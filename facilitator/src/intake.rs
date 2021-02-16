use crate::{
    batch::{Batch, BatchReader, BatchWriter},
    idl::{IngestionDataSharePacket, IngestionHeader, Packet, ValidationHeader, ValidationPacket},
    metrics::IntakeMetricsCollector,
    transport::{SignableTransport, VerifiableAndDecryptableTransport},
    BatchSigningKey, Error,
};
use anyhow::{anyhow, ensure, Context, Result};
use chrono::NaiveDateTime;
use log::info;
use prio::{encrypt::PrivateKey, finite_field::Field, server::Server};
use ring::signature::UnparsedPublicKey;
use std::{collections::HashMap, convert::TryFrom, iter::Iterator};
use uuid::Uuid;

/// BatchIntaker is responsible for validating a batch of data packet shares
/// sent by the ingestion server and emitting validation shares to the other
/// share processor.
pub struct BatchIntaker<'a> {
    intake_batch: BatchReader<'a, IngestionHeader, IngestionDataSharePacket>,
    intake_public_keys: &'a HashMap<String, UnparsedPublicKey<Vec<u8>>>,
    packet_decryption_keys: &'a Vec<PrivateKey>,
    peer_validation_batch: BatchWriter<'a, ValidationHeader, ValidationPacket>,
    peer_validation_batch_signing_key: &'a BatchSigningKey,
    own_validation_batch: BatchWriter<'a, ValidationHeader, ValidationPacket>,
    own_validation_batch_signing_key: &'a BatchSigningKey,
    is_first: bool,
    callback_cadence: u32,
    metrics_collector: Option<&'a IntakeMetricsCollector>,
    use_bogus_packet_file_digest: bool,
}

impl<'a> BatchIntaker<'a> {
    pub fn new(
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        ingestion_transport: &'a mut VerifiableAndDecryptableTransport,
        own_validation_transport: &'a mut SignableTransport,
        peer_validation_transport: &'a mut SignableTransport,
        is_first: bool,
    ) -> Result<BatchIntaker<'a>> {
        Ok(BatchIntaker {
            intake_batch: BatchReader::new(
                Batch::new_ingestion(aggregation_name, batch_id, date),
                &mut *ingestion_transport.transport.transport,
            ),
            intake_public_keys: &ingestion_transport.transport.batch_signing_public_keys,
            packet_decryption_keys: &ingestion_transport.packet_decryption_keys,
            peer_validation_batch: BatchWriter::new(
                Batch::new_validation(aggregation_name, batch_id, date, is_first),
                &mut *peer_validation_transport.transport,
            ),
            own_validation_batch: BatchWriter::new(
                Batch::new_validation(aggregation_name, batch_id, date, is_first),
                &mut *own_validation_transport.transport,
            ),
            peer_validation_batch_signing_key: &peer_validation_transport.batch_signing_key,
            own_validation_batch_signing_key: &own_validation_transport.batch_signing_key,
            is_first,
            callback_cadence: 1000,
            metrics_collector: None,
            use_bogus_packet_file_digest: false,
        })
    }

    /// Set the cadence at which the callback passed to
    /// generate_validation_share is invoked, i.e., after how many processed
    /// packets. This function is not safe to call while a call to
    /// generate_validation_share is in flight and is intended only for testing.
    pub fn set_callback_cadence(&mut self, cadence: u32) {
        self.callback_cadence = cadence;
    }

    /// Provide a collector in which metrics about this intake task will be
    /// recorded.
    pub fn set_metrics_collector(&mut self, collector: &'a IntakeMetricsCollector) {
        self.metrics_collector = Some(collector);
    }

    /// Sets whether this BatchIntaker will use a bogus value for the packet
    /// file digest when constructing the header of a validation batch. This is
    /// intended only for testing.
    pub fn set_use_bogus_packet_file_digest(&mut self, bogus: bool) {
        self.use_bogus_packet_file_digest = bogus;
    }

    /// Fetches the ingestion batch, validates the signatures over its header
    /// and packet file, then computes validation shares and sends them to the
    /// peer share processor. The provided callback is invoked once for every
    /// thousand processed packets, unless set_callback_cadence has been called.
    pub fn generate_validation_share<F: FnMut()>(&mut self, mut callback: F) -> Result<()> {
        info!(
            "processing intake from {} and saving validity to {} and {}",
            self.intake_batch.path(),
            self.own_validation_batch.path(),
            self.peer_validation_batch.path()
        );

        let ingestion_header = self.intake_batch.header(self.intake_public_keys)?;
        ensure!(
            ingestion_header.bins > 0,
            "invalid bin count {}",
            ingestion_header.bins
        );

        // Ideally, we would use the encryption_key_id in the ingestion packet
        // to figure out which private key to use for decryption, but that field
        // is optional. Instead we try all the keys we have available until one
        // works.
        // https://github.com/abetterinternet/prio-server/issues/73
        let mut servers = self
            .packet_decryption_keys
            .iter()
            .map(|k| Server::new(ingestion_header.bins as usize, self.is_first, k.clone()))
            .collect::<Vec<Server>>();

        // Read all the ingestion packets, generate a verification message for
        // each, and write them to the validation batch.
        let mut ingestion_packet_reader =
            self.intake_batch.packet_file_reader(&ingestion_header)?;

        let mut processed_packets = 0;
        // Copy of callback_cadence that's safe to use in the closure
        let callback_cadence = self.callback_cadence;

        let packet_file_digest = self.peer_validation_batch.multi_packet_file_writer(
            vec![&mut self.own_validation_batch],
            |mut packet_writer| loop {
                let packet = match IngestionDataSharePacket::read(&mut ingestion_packet_reader) {
                    Ok(p) => p,
                    Err(Error::Eof) => return Ok(()),
                    Err(e) => return Err(e.into()),
                };

                let r_pit = u32::try_from(packet.r_pit)
                    .with_context(|| format!("illegal r_pit value {}", packet.r_pit))?;

                // TODO(timg): if this fails for a non-empty subset of the
                // ingestion packets, do we abort handling of the entire
                // batch (as implemented currently) or should we record it
                // as an invalid UUID and emit a validation batch for the
                // other packets?
                let mut did_create_validation_packet = false;
                for server in servers.iter_mut() {
                    let validation_message = match server.generate_verification_message(
                        Field::from(r_pit),
                        &packet.encrypted_payload,
                    ) {
                        Some(m) => m,
                        None => continue,
                    };

                    let packet = ValidationPacket {
                        uuid: packet.uuid,
                        f_r: u32::from(validation_message.f_r) as i64,
                        g_r: u32::from(validation_message.g_r) as i64,
                        h_r: u32::from(validation_message.h_r) as i64,
                    };
                    packet.write(&mut packet_writer)?;
                    did_create_validation_packet = true;
                    break;
                }
                if !did_create_validation_packet {
                    return Err(anyhow!("failed to construct validation message"));
                }
                processed_packets += 1;
                if processed_packets % callback_cadence == 0 {
                    callback();
                }
            },
        )?;

        // If the caller requested it, we insert a bogus packet file digest into
        // the own and peer validaton batch headers instead of the real computed
        // digest. This is meant to simulate a buggy peer data share processor,
        // so that we can test how the aggregation step behaves.
        let packet_file_digest = if self.use_bogus_packet_file_digest {
            info!("using bogus packet file digest");
            vec![0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8]
        } else {
            packet_file_digest.as_ref().to_vec()
        };

        // Construct validation header and write it out
        let header = ValidationHeader {
            batch_uuid: ingestion_header.batch_uuid,
            name: ingestion_header.name,
            bins: ingestion_header.bins,
            epsilon: ingestion_header.epsilon,
            prime: ingestion_header.prime,
            number_of_servers: ingestion_header.number_of_servers,
            hamming_weight: ingestion_header.hamming_weight,
            packet_file_digest,
        };
        let peer_header_signature = self
            .peer_validation_batch
            .put_header(&header, &self.peer_validation_batch_signing_key.key)?;
        let own_header_signature = self
            .own_validation_batch
            .put_header(&header, &self.own_validation_batch_signing_key.key)?;

        // Construct and write out signature
        self.peer_validation_batch.put_signature(
            &peer_header_signature,
            &self.peer_validation_batch_signing_key.identifier,
        )?;
        Ok(self.own_validation_batch.put_signature(
            &own_header_signature,
            &self.own_validation_batch_signing_key.identifier,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sample::{generate_ingestion_sample, SampleOutput},
        test_utils::{
            default_facilitator_signing_private_key, default_ingestor_private_key,
            default_ingestor_public_key, default_pha_signing_private_key,
            DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY, DEFAULT_PHA_ECIES_PRIVATE_KEY,
        },
        transport::{LocalFileTransport, SignableTransport, VerifiableTransport},
    };

    #[test]
    fn share_validator() {
        let pha_tempdir = tempfile::TempDir::new().unwrap();
        let pha_copy_tempdir = tempfile::TempDir::new().unwrap();
        let facilitator_tempdir = tempfile::TempDir::new().unwrap();
        let facilitator_copy_tempdir = tempfile::TempDir::new().unwrap();

        let aggregation_name = "fake-aggregation-1".to_owned();
        let date = NaiveDateTime::from_timestamp(1234567890, 654321);
        let batch_uuid = Uuid::new_v4();

        let mut pha_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_key: PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            drop_nth_packet: None,
        };

        let mut facilitator_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    facilitator_tempdir.path().to_path_buf(),
                )),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_key: PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY)
                .unwrap(),
            drop_nth_packet: None,
        };

        generate_ingestion_sample(
            &batch_uuid,
            &aggregation_name,
            &date,
            10,
            10,
            0.11,
            100,
            100,
            &mut pha_output,
            &mut facilitator_output,
        )
        .expect("failed to generate sample");

        let mut ingestor_pub_keys = HashMap::new();
        ingestor_pub_keys.insert(
            default_ingestor_private_key().identifier,
            default_ingestor_public_key(),
        );
        let mut pha_ingest_transport = VerifiableAndDecryptableTransport {
            transport: VerifiableTransport {
                transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
                batch_signing_public_keys: ingestor_pub_keys.clone(),
            },
            packet_decryption_keys: vec![
                PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap()
            ],
        };

        let mut facilitator_ingest_transport = VerifiableAndDecryptableTransport {
            transport: VerifiableTransport {
                transport: Box::new(LocalFileTransport::new(
                    facilitator_tempdir.path().to_path_buf(),
                )),
                batch_signing_public_keys: ingestor_pub_keys.clone(),
            },
            packet_decryption_keys: vec![PrivateKey::from_base64(
                DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY,
            )
            .unwrap()],
        };

        let mut pha_peer_validate_transport = SignableTransport {
            transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
            batch_signing_key: default_pha_signing_private_key(),
        };

        let mut facilitator_peer_validate_transport = SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_tempdir.path().to_path_buf(),
            )),
            batch_signing_key: default_facilitator_signing_private_key(),
        };

        let mut pha_own_validate_transport = SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                pha_copy_tempdir.path().to_path_buf(),
            )),
            batch_signing_key: default_pha_signing_private_key(),
        };

        let mut facilitator_own_validate_transport = SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_copy_tempdir.path().to_path_buf(),
            )),
            batch_signing_key: default_facilitator_signing_private_key(),
        };

        let mut pha_ingestor = BatchIntaker::new(
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut pha_ingest_transport,
            &mut pha_peer_validate_transport,
            &mut pha_own_validate_transport,
            true,
        )
        .unwrap();

        pha_ingestor
            .generate_validation_share(|| {})
            .expect("PHA failed to generate validation");

        let mut facilitator_ingestor = BatchIntaker::new(
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut facilitator_ingest_transport,
            &mut facilitator_peer_validate_transport,
            &mut facilitator_own_validate_transport,
            false,
        )
        .unwrap();

        facilitator_ingestor
            .generate_validation_share(|| {})
            .expect("facilitator failed to generate validation");
    }
}
