use crate::{
    batch::{BatchIO, BatchReader, BatchWriter},
    idl::{IngestionDataSharePacket, IngestionHeader, Packet, ValidationHeader, ValidationPacket},
    transport::Transport,
    Error,
};
use chrono::NaiveDateTime;
use prio::{encrypt::PrivateKey, finite_field::Field, server::Server};
use ring::signature::{EcdsaKeyPair, UnparsedPublicKey};
use std::convert::TryFrom;
use uuid::Uuid;

/// BatchIntaker is responsible for validating a batch of data packet shares
/// sent by the ingestion server and emitting validation shares to the other
/// share processor.
pub struct BatchIntaker<'a> {
    ingestion_batch: BatchReader<'a, IngestionHeader, IngestionDataSharePacket>,
    validation_batch: BatchWriter<'a, ValidationHeader, ValidationPacket>,
    is_first: bool,
    share_processor_ecies_key: &'a PrivateKey,
    share_processor_signing_key: &'a EcdsaKeyPair,
    ingestor_key: &'a UnparsedPublicKey<Vec<u8>>,
}

impl<'a> BatchIntaker<'a> {
    pub fn new(
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        ingestion_transport: &'a mut dyn Transport,
        validation_transport: &'a mut dyn Transport,
        is_first: bool,
        share_processor_ecies_key: &'a PrivateKey,
        share_processor_signing_key: &'a EcdsaKeyPair,
        ingestor_key: &'a UnparsedPublicKey<Vec<u8>>,
    ) -> Result<BatchIntaker<'a>, Error> {
        Ok(BatchIntaker {
            ingestion_batch: BatchReader::new_ingestion(
                aggregation_name.clone(),
                batch_id,
                date,
                ingestion_transport,
            )?,
            validation_batch: BatchWriter::new_validation(
                aggregation_name,
                batch_id,
                date,
                is_first,
                validation_transport,
            )?,
            is_first: is_first,
            share_processor_ecies_key: share_processor_ecies_key,
            share_processor_signing_key: share_processor_signing_key,
            ingestor_key: ingestor_key,
        })
    }

    /// Fetches the ingestion batch, validates the signatures over its header
    /// and packet file, then computes validation shares and sends them to the
    /// peer share processor.
    pub fn generate_validation_share(&mut self) -> Result<(), Error> {
        let ingestion_header = self.ingestion_batch.header(&self.ingestor_key)?;
        if ingestion_header.bins <= 0 {
            return Err(Error::MalformedHeaderError(format!(
                "invalid bins/dimension value {}",
                ingestion_header.bins
            )));
        }

        let mut server = Server::new(
            ingestion_header.bins as usize,
            self.is_first,
            self.share_processor_ecies_key.clone(),
        );

        // Read all the ingestion packets, generate a verification message for
        // each, and write them to the validation batch.
        let mut ingestion_packet_reader =
            self.ingestion_batch.packet_file_reader(&ingestion_header)?;

        let packet_file_digest =
            self.validation_batch
                .packet_file_writer(|mut packet_writer| loop {
                    let packet = match IngestionDataSharePacket::read(&mut ingestion_packet_reader)
                    {
                        Ok(p) => p,
                        Err(Error::EofError) => return Ok(()),
                        Err(e) => return Err(e),
                    };

                    let r_pit = match u32::try_from(packet.r_pit) {
                        Ok(v) => v,
                        Err(s) => {
                            return Err(Error::MalformedDataPacketError(format!(
                                "illegal r_pit value {} ({})",
                                packet.r_pit, s
                            )))
                        }
                    };

                    // TODO(timg): if this fails for a non-empty subset of the
                    // ingestion packets, do we abort handling of the entire
                    // batch (as implemented currently) or should we record it
                    // as an invalid UUID and emit a validation batch for the
                    //  other packets?
                    let validation_message = match server.generate_verification_message(
                        Field::from(r_pit),
                        &packet.encrypted_payload,
                    ) {
                        Some(m) => m,
                        None => {
                            return Err(Error::LibPrioError(
                                "failed to construct validation message".to_owned(),
                                None,
                            ))
                        }
                    };

                    let packet = ValidationPacket {
                        uuid: packet.uuid,
                        f_r: u32::from(validation_message.f_r) as i64,
                        g_r: u32::from(validation_message.g_r) as i64,
                        h_r: u32::from(validation_message.h_r) as i64,
                    };
                    packet.write(&mut packet_writer)?;
                })?;

        // Construct validation header and write it out
        let header_signature = self.validation_batch.put_header(
            &ValidationHeader {
                batch_uuid: ingestion_header.batch_uuid,
                name: ingestion_header.name,
                bins: ingestion_header.bins,
                epsilon: ingestion_header.epsilon,
                prime: ingestion_header.prime,
                number_of_servers: ingestion_header.number_of_servers,
                hamming_weight: ingestion_header.hamming_weight,
                packet_file_digest: packet_file_digest.as_ref().to_vec(),
            },
            &self.share_processor_signing_key,
        )?;

        // Construct and write out signature
        self.validation_batch.put_signature(&header_signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sample::generate_ingestion_sample,
        test_utils::{
            default_facilitator_signing_private_key, default_ingestor_private_key,
            default_ingestor_private_key_raw, default_pha_signing_private_key,
            DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY, DEFAULT_PHA_ECIES_PRIVATE_KEY,
        },
        transport::LocalFileTransport,
    };
    use ring::signature::{KeyPair, ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING};

    #[test]
    fn share_validator() {
        let pha_tempdir = tempfile::TempDir::new().unwrap();
        let facilitator_tempdir = tempfile::TempDir::new().unwrap();

        let aggregation_name = "fake-aggregation-1".to_owned();
        let date = NaiveDateTime::from_timestamp(1234567890, 654321);
        let batch_uuid = Uuid::new_v4();
        let mut pha_ingest_transport = LocalFileTransport::new(pha_tempdir.path().to_path_buf());
        let mut facilitator_ingest_transport =
            LocalFileTransport::new(facilitator_tempdir.path().to_path_buf());
        let mut pha_validate_transport = LocalFileTransport::new(pha_tempdir.path().to_path_buf());
        let mut facilitator_validate_transport =
            LocalFileTransport::new(facilitator_tempdir.path().to_path_buf());

        let pha_ecies_key = PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap();
        let facilitator_ecies_key =
            PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap();
        let ingestor_pub_key = UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_FIXED,
            default_ingestor_private_key()
                .public_key()
                .as_ref()
                .to_vec(),
        );
        let pha_signing_key = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &default_pha_signing_private_key(),
        )
        .unwrap();
        let facilitator_signing_key = default_facilitator_signing_private_key();

        generate_ingestion_sample(
            &mut pha_ingest_transport,
            &mut facilitator_ingest_transport,
            &batch_uuid,
            &aggregation_name,
            &date,
            &pha_ecies_key,
            &facilitator_ecies_key,
            &default_ingestor_private_key_raw(),
            10,
            10,
            0.11,
            100,
            100,
        )
        .expect("failed to generate sample");

        let mut pha_ingestor = BatchIntaker::new(
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut pha_ingest_transport,
            &mut pha_validate_transport,
            true,
            &pha_ecies_key,
            &pha_signing_key,
            &ingestor_pub_key,
        )
        .unwrap();

        pha_ingestor
            .generate_validation_share()
            .expect("PHA failed to generate validation");

        let mut facilitator_ingestor = BatchIntaker::new(
            &aggregation_name,
            &batch_uuid,
            &date,
            &mut facilitator_ingest_transport,
            &mut facilitator_validate_transport,
            false,
            &facilitator_ecies_key,
            &facilitator_signing_key,
            &ingestor_pub_key,
        )
        .unwrap();

        facilitator_ingestor
            .generate_validation_share()
            .expect("facilitator failed to generate validation");
    }
}
