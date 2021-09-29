use crate::{
    batch::{Batch, BatchReader, BatchWriter},
    generate_validation_packet,
    idl::{IngestionDataSharePacket, IngestionHeader, Packet, ValidationHeader, ValidationPacket},
    logging::event,
    metrics::IntakeMetricsCollector,
    transport::{SignableTransport, VerifiableAndDecryptableTransport},
    BatchSigningKey, Error, DATE_FORMAT,
};
use anyhow::{ensure, Result};
use chrono::NaiveDateTime;
use prio::{
    encrypt::{PrivateKey, PublicKey},
    field::Field32,
    server::Server,
};
use ring::signature::UnparsedPublicKey;
use slog::{debug, info, o, Logger};
use std::{collections::HashMap, iter::Iterator};
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
    is_first: bool,
    callback_cadence: u32,
    metrics_collector: Option<&'a IntakeMetricsCollector>,
    use_bogus_packet_file_digest: bool,
    logger: Logger,
}

impl<'a> BatchIntaker<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        trace_id: &'a Uuid,
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        ingestion_transport: &'a mut VerifiableAndDecryptableTransport,
        peer_validation_transport: &'a mut SignableTransport,
        is_first: bool,
        permit_malformed_batch: bool,
        parent_logger: &Logger,
    ) -> Result<BatchIntaker<'a>> {
        let logger = parent_logger.new(o!(
            event::TRACE_ID => trace_id.to_string(),
            event::AGGREGATION_NAME => aggregation_name.to_owned(),
            event::BATCH_ID => batch_id.to_string(),
            event::BATCH_DATE => date.format(DATE_FORMAT).to_string(),
            event::INGESTION_PATH => ingestion_transport.transport.transport.path(),
            event::PEER_VALIDATION_PATH => peer_validation_transport.transport.path(),
        ));
        Ok(BatchIntaker {
            intake_batch: BatchReader::new(
                Batch::new_ingestion(aggregation_name, batch_id, date),
                &mut *ingestion_transport.transport.transport,
                permit_malformed_batch,
                trace_id,
                &logger,
            ),
            intake_public_keys: &ingestion_transport.transport.batch_signing_public_keys,
            packet_decryption_keys: &ingestion_transport.packet_decryption_keys,
            peer_validation_batch: BatchWriter::new(
                Batch::new_validation(aggregation_name, batch_id, date, is_first),
                &mut *peer_validation_transport.transport,
                trace_id,
            ),
            peer_validation_batch_signing_key: &peer_validation_transport.batch_signing_key,
            is_first,
            callback_cadence: 1000,
            metrics_collector: None,
            use_bogus_packet_file_digest: false,
            logger,
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
    pub fn generate_validation_share<F>(&mut self, mut callback: F) -> Result<()>
    where
        F: FnMut(&Logger),
    {
        info!(self.logger, "processing batch intake task");

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
        let mut servers: Vec<Server<Field32>> = self
            .packet_decryption_keys
            .iter()
            .map(|k| {
                debug!(
                    self.logger,
                    "Public key for server is: {:?}",
                    PublicKey::from(k)
                );
                Server::new(ingestion_header.bins as usize, self.is_first, k.clone())
            })
            .collect();

        debug!(self.logger, "We have {} servers.", &servers.len());

        // Read all the ingestion packets, generate a verification message for
        // each, and write them to the validation batch.
        let mut ingestion_packet_reader =
            self.intake_batch.packet_file_reader(&ingestion_header)?;

        let mut processed_packets = 0;
        // Borrowing distinct parts of a struct works, but not under closures:
        // https://github.com/rust-lang/rust/issues/53488
        // The workaround is to borrow or copy fields outside the closure.
        let callback_cadence = self.callback_cadence;
        let logger = &self.logger;

        let packet_file_digest =
            self.peer_validation_batch
                .packet_file_writer(|mut packet_writer| loop {
                    let packet = match IngestionDataSharePacket::read(&mut ingestion_packet_reader)
                    {
                        Ok(p) => p,
                        Err(Error::EofError) => return Ok(()),
                        Err(e) => return Err(e.into()),
                    };
                    let validation_packet = generate_validation_packet(&mut servers, &packet)?;
                    validation_packet.write(&mut packet_writer)?;
                    processed_packets += 1;
                    if processed_packets % callback_cadence == 0 {
                        callback(logger);
                    }
                })?;

        // If the caller requested it, we insert a bogus packet file digest into
        // the own and peer validaton batch headers instead of the real computed
        // digest. This is meant to simulate a buggy peer data share processor,
        // so that we can test how the aggregation step behaves.
        let packet_file_digest = if self.use_bogus_packet_file_digest {
            info!(self.logger, "using bogus packet file digest");
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

        // Construct and write out signature
        self.peer_validation_batch.put_signature(
            &peer_header_signature,
            &self.peer_validation_batch_signing_key.identifier,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        logging::setup_test_logging,
        sample::{SampleGenerator, SampleOutput},
        test_utils::{
            default_facilitator_signing_private_key, default_ingestor_private_key,
            default_ingestor_public_key, default_packet_encryption_certificate_signing_request,
            default_pha_signing_private_key,
            DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST_PRIVATE_KEY,
            DEFAULT_PHA_ECIES_PRIVATE_KEY, DEFAULT_TRACE_ID,
        },
        transport::{LocalFileTransport, SignableTransport, VerifiableTransport},
    };
    use assert_matches::assert_matches;
    use prio::{encrypt::PublicKey, server::ServerError, util::SerializeError};

    #[test]
    fn share_validator() {
        let logger = setup_test_logging();
        let pha_tempdir = tempfile::TempDir::new().unwrap();
        let facilitator_tempdir = tempfile::TempDir::new().unwrap();

        let aggregation_name = "fake-aggregation-1".to_owned();
        let date = NaiveDateTime::from_timestamp(1234567890, 654321);
        let batch_uuid = Uuid::new_v4();

        let packet_encryption_csr = default_packet_encryption_certificate_signing_request();

        let mut pha_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from_base64(
                &packet_encryption_csr.base64_public_key().unwrap(),
            )
            .unwrap(),
            drop_nth_packet: None,
        };

        let mut facilitator_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    facilitator_tempdir.path().to_path_buf(),
                )),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from_base64(
                &packet_encryption_csr.base64_public_key().unwrap(),
            )
            .unwrap(),
            drop_nth_packet: None,
        };

        let mut sample_generator = SampleGenerator::new(
            &aggregation_name,
            10,
            0.11,
            100,
            100,
            &mut pha_output,
            &mut facilitator_output,
            &logger,
        );

        sample_generator
            .generate_ingestion_sample(&DEFAULT_TRACE_ID, &batch_uuid, &date, 10)
            .unwrap();

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
            packet_decryption_keys: vec![PrivateKey::from_base64(
                DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST_PRIVATE_KEY,
            )
            .unwrap()],
        };

        let mut facilitator_ingest_transport = VerifiableAndDecryptableTransport {
            transport: VerifiableTransport {
                transport: Box::new(LocalFileTransport::new(
                    facilitator_tempdir.path().to_path_buf(),
                )),
                batch_signing_public_keys: ingestor_pub_keys,
            },
            packet_decryption_keys: vec![PrivateKey::from_base64(
                DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST_PRIVATE_KEY,
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

        let mut pha_ingestor = BatchIntaker::new(
            &DEFAULT_TRACE_ID,
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut pha_ingest_transport,
            &mut pha_peer_validate_transport,
            true,
            false,
            &logger,
        )
        .unwrap();

        pha_ingestor
            .generate_validation_share(|_| {})
            .expect("PHA failed to generate validation");

        let mut facilitator_ingestor = BatchIntaker::new(
            &DEFAULT_TRACE_ID,
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut facilitator_ingest_transport,
            &mut facilitator_peer_validate_transport,
            false,
            false,
            &logger,
        )
        .unwrap();

        facilitator_ingestor
            .generate_validation_share(|_| {})
            .expect("facilitator failed to generate validation");
    }

    #[test]
    fn wrong_decryption_key() {
        let logger = setup_test_logging();
        let pha_tempdir = tempfile::TempDir::new().unwrap();
        let facilitator_tempdir = tempfile::TempDir::new().unwrap();

        let aggregation_name = "fake-aggregation-1".to_owned();
        let date = NaiveDateTime::from_timestamp(1234567890, 654321);
        let batch_uuid = Uuid::new_v4();

        let packet_encryption_csr = default_packet_encryption_certificate_signing_request();

        let mut pha_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from_base64(
                &packet_encryption_csr.base64_public_key().unwrap(),
            )
            .unwrap(),
            drop_nth_packet: None,
        };

        let mut facilitator_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    facilitator_tempdir.path().to_path_buf(),
                )),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from_base64(
                &packet_encryption_csr.base64_public_key().unwrap(),
            )
            .unwrap(),
            drop_nth_packet: None,
        };

        let mut sample_generator = SampleGenerator::new(
            &aggregation_name,
            10,
            0.11,
            100,
            100,
            &mut pha_output,
            &mut facilitator_output,
            &logger,
        );

        sample_generator
            .generate_ingestion_sample(&DEFAULT_TRACE_ID, &batch_uuid, &date, 10)
            .unwrap();

        let mut ingestor_pub_keys = HashMap::new();
        ingestor_pub_keys.insert(
            default_ingestor_private_key().identifier,
            default_ingestor_public_key(),
        );
        let mut pha_ingest_transport = VerifiableAndDecryptableTransport {
            transport: VerifiableTransport {
                transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
                batch_signing_public_keys: ingestor_pub_keys,
            },
            packet_decryption_keys: vec![
                PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap()
            ],
        };

        let mut pha_peer_validate_transport = SignableTransport {
            transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
            batch_signing_key: default_pha_signing_private_key(),
        };

        let mut pha_ingestor = BatchIntaker::new(
            &DEFAULT_TRACE_ID,
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut pha_ingest_transport,
            &mut pha_peer_validate_transport,
            true,
            false,
            &logger,
        )
        .unwrap();

        let err = pha_ingestor.generate_validation_share(|_| {}).unwrap_err();
        assert!(err
            .to_string()
            .contains("failed to construct validation message for packet",));
    }

    #[test]
    fn wrong_packet_dimension() {
        let logger = setup_test_logging();
        let pha_tempdir = tempfile::TempDir::new().unwrap();
        let facilitator_tempdir = tempfile::TempDir::new().unwrap();

        let aggregation_name = "fake-aggregation-1".to_owned();
        let date = NaiveDateTime::from_timestamp(1234567890, 654321);
        let batch_uuid = Uuid::new_v4();

        let packet_encryption_csr = default_packet_encryption_certificate_signing_request();

        let mut pha_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from_base64(
                &packet_encryption_csr.base64_public_key().unwrap(),
            )
            .unwrap(),
            drop_nth_packet: None,
        };

        let mut facilitator_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    facilitator_tempdir.path().to_path_buf(),
                )),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from_base64(
                &packet_encryption_csr.base64_public_key().unwrap(),
            )
            .unwrap(),
            drop_nth_packet: None,
        };

        let mut sample_generator = SampleGenerator::new(
            &aggregation_name,
            10,
            0.11,
            100,
            100,
            &mut pha_output,
            &mut facilitator_output,
            &logger,
        );
        sample_generator.set_generate_short_packet(5);

        sample_generator
            .generate_ingestion_sample(&DEFAULT_TRACE_ID, &batch_uuid, &date, 10)
            .unwrap();

        let mut ingestor_pub_keys = HashMap::new();
        ingestor_pub_keys.insert(
            default_ingestor_private_key().identifier,
            default_ingestor_public_key(),
        );
        let mut pha_ingest_transport = VerifiableAndDecryptableTransport {
            transport: VerifiableTransport {
                transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
                batch_signing_public_keys: ingestor_pub_keys,
            },
            packet_decryption_keys: vec![PrivateKey::from_base64(
                DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST_PRIVATE_KEY,
            )
            .unwrap()],
        };

        let mut pha_peer_validate_transport = SignableTransport {
            transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
            batch_signing_key: default_pha_signing_private_key(),
        };

        let mut pha_ingestor = BatchIntaker::new(
            &DEFAULT_TRACE_ID,
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut pha_ingest_transport,
            &mut pha_peer_validate_transport,
            true,
            false,
            &logger,
        )
        .unwrap();

        let err = pha_ingestor.generate_validation_share(|_| {}).unwrap_err();
        assert_matches!(
            err.downcast(),
            Ok(ServerError::Serialize(
                SerializeError::UnpackInputSizeMismatch
            ))
        );
    }
}
