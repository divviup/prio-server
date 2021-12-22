use crate::{
    idl::{
        BatchSignature, Header, IngestionDataSharePacket, IngestionHeader, InvalidPacket, Packet,
        SumPart, ValidationHeader, ValidationPacket,
    },
    metrics::BatchReaderMetricsCollector,
    transport::{Transport, TransportWriter},
    DigestWriter, Error, DATE_FORMAT,
};
use anyhow::{anyhow, Context, Result};
use avro_rs::{Reader, Writer};
use chrono::NaiveDateTime;
use ring::{
    digest::Digest,
    rand::SystemRandom,
    signature::{EcdsaKeyPair, Signature, UnparsedPublicKey},
};
use slog::{o, warn, Logger};
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    marker::PhantomData,
};
use uuid::Uuid;

pub const AGGREGATION_DATE_FORMAT: &str = "%Y%m%d%H%M";

/// Manages the paths to the different files in a batch
pub struct Batch<H: Header, P: Packet> {
    header_path: String,
    signature_path: String,
    packet_file_path: String,

    // These next two fields are not real and are used because not using H and P
    // in the struct definition is an error.
    phantom_header: PhantomData<*const H>,
    phantom_packet: PhantomData<*const P>,
}

impl Batch<IngestionHeader, IngestionDataSharePacket> {
    /// Creates a Batch representing an ingestion batch
    pub fn new_ingestion(aggregation_name: &str, batch_id: &Uuid, date: &NaiveDateTime) -> Self {
        Self::new(aggregation_name, batch_id, date, "batch")
    }
}

impl Batch<ValidationHeader, ValidationPacket> {
    /// Creates a Batch representing a validation batch
    pub fn new_validation(
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        is_first: bool,
    ) -> Self {
        Self::new(
            aggregation_name,
            batch_id,
            date,
            &format!("validity_{}", if is_first { 0 } else { 1 }),
        )
    }
}

impl Batch<SumPart, InvalidPacket> {
    // Creates a batch representing a sum part batch
    pub fn new_sum(
        instance_name: &str,
        aggregation_name: &str,
        aggregation_start: &NaiveDateTime,
        aggregation_end: &NaiveDateTime,
        is_first: bool,
    ) -> Self {
        let batch_path = format!(
            "{}/{}/{}-{}",
            instance_name,
            aggregation_name,
            aggregation_start.format(AGGREGATION_DATE_FORMAT),
            aggregation_end.format(AGGREGATION_DATE_FORMAT)
        );
        let filename = format!("sum_{}", if is_first { 0 } else { 1 });

        Self {
            header_path: format!("{}.{}", batch_path, filename),
            signature_path: format!("{}.{}.sig", batch_path, filename),
            packet_file_path: format!(
                "{}.invalid_uuid_{}.avro",
                batch_path,
                if is_first { 0 } else { 1 }
            ),

            phantom_header: PhantomData,
            phantom_packet: PhantomData,
        }
    }
}

impl<H: Header, P: Packet> Batch<H, P> {
    fn new(aggregation_name: &str, batch_id: &Uuid, date: &NaiveDateTime, filename: &str) -> Self {
        let batch_path = format!(
            "{}/{}/{}",
            aggregation_name,
            date.format(DATE_FORMAT),
            batch_id.to_hyphenated()
        );
        Self {
            header_path: format!("{}.{}", batch_path, filename),
            signature_path: format!("{}.{}.sig", batch_path, filename),
            packet_file_path: format!("{}.{}.avro", batch_path, filename),
            phantom_header: PhantomData,
            phantom_packet: PhantomData,
        }
    }

    fn header_key(&self) -> &str {
        self.header_path.as_ref()
    }

    fn signature_key(&self) -> &str {
        self.signature_path.as_ref()
    }

    fn packet_file_key(&self) -> &str {
        self.packet_file_path.as_ref()
    }
}

/// Allows reading files, including signature validation, from an ingestion or
/// validation batch containing a header, a packet file and a signature.
pub struct BatchReader<'a, H: Header, P: Packet> {
    trace_id: &'a Uuid,
    batch: Batch<H, P>,
    transport: &'a dyn Transport,
    permit_malformed_batch: bool,
    metrics_collector: Option<&'a BatchReaderMetricsCollector>,
    logger: Logger,
}

impl<'a, H: Header, P: Packet> BatchReader<'a, H, P> {
    /// Create a new BatchReader which will fetch the specified batch from the
    /// provided transport. If permissive is true, then invalid signatures or
    /// packet file digest mismatches will be ignored.
    pub fn new(
        batch: Batch<H, P>,
        transport: &'a dyn Transport,
        permit_malformed_batch: bool,
        trace_id: &'a Uuid,
        parent_logger: &Logger,
    ) -> Self {
        let logger = parent_logger.new(o!(
            "batch" => batch.header_key().to_owned(),
            "transport" => transport.path(),
        ));
        BatchReader {
            trace_id,
            batch,
            transport,
            permit_malformed_batch,
            metrics_collector: None,
            logger,
        }
    }

    pub fn set_metrics_collector(&mut self, collector: &'a BatchReaderMetricsCollector) {
        self.metrics_collector = Some(collector);
    }

    pub fn path(&self) -> String {
        self.transport.path()
    }

    // Returns the parsed header & packets from this batch, but only if the
    // signature & digest are valid.
    pub fn read(
        &self,
        public_keys: &HashMap<String, UnparsedPublicKey<Vec<u8>>>,
    ) -> Result<(H, Vec<P>)> {
        // First, read the batch signature.
        let signature = BatchSignature::read(
            self.transport
                .get(self.batch.signature_key(), self.trace_id)?,
        )?;

        // Read the header, and check the header's signature.
        let mut header_buf = Vec::new();
        self.transport
            .get(self.batch.header_key(), self.trace_id)?
            .read_to_end(&mut header_buf)
            .context("failed to read header from transport")?;

        if let Err(err) = public_keys
            .get(&signature.key_identifier)
            .with_context(|| {
                format!(
                    "key identifier {} not present in key map {:?}",
                    signature.key_identifier,
                    public_keys.keys(),
                )
            })?
            .verify(&header_buf, &signature.batch_header_signature)
        {
            if let Some(collector) = self.metrics_collector {
                collector
                    .invalid_validation_batches
                    .with_label_values(&["header"])
                    .inc();
            }
            let message = format!(
                "invalid signature on header with key {}: {:?}",
                signature.key_identifier, err
            );
            if self.permit_malformed_batch {
                warn!(self.logger, "{}", message);
            } else {
                return Err(anyhow!("{}", message));
            }
        }
        let header = H::read(Cursor::new(header_buf))?;

        // Read packets, after verifying that digest matches header.
        // We read all of the packets, rather than streaming bytes as we
        // need them to yield a packet, because we need to check that the
        // header's recorded digest matches the actual content's digest. The
        // ingestion server authors suggest that the maximum batch size will be
        // ~400MiB, so this should work out okay.
        let mut packet_reader = self
            .transport
            .get(self.batch.packet_file_key(), self.trace_id)?;
        let mut packet_bytes = Vec::new();
        let mut digest_writer = DigestWriter::new(&mut packet_bytes);

        std::io::copy(&mut packet_reader, &mut digest_writer)
            .context("failed to load packet file")?;

        let packet_digest = digest_writer.finish();
        if header.packet_file_digest() != packet_digest.as_ref() {
            if let Some(collector) = self.metrics_collector {
                collector
                    .invalid_validation_batches
                    .with_label_values(&["packet_file"])
                    .inc();
            }
            let message = format!(
                "packet file digest in header {} does not match actual packet file digest {}",
                hex::encode(header.packet_file_digest()),
                hex::encode(packet_digest)
            );
            if self.permit_malformed_batch {
                warn!(self.logger, "{}", message);
            } else {
                return Err(anyhow!("{}", message));
            }
        }

        // avro_rs readers will always attempt to read a header on creation;
        // apparently, avro_rs writers don't bother writing a header unless at
        // least one record is written. So we need to special-case zero bytes
        // of content as returning an empty vector of packets, because trying
        // to use a Reader will error out.
        let packets =
            if packet_bytes.is_empty() {
                Vec::new()
            } else {
                Reader::with_schema(P::schema(), Cursor::new(&packet_bytes))
                    .context("failed to create Avro reader for packets")?
                    .map(|val| {
                        P::try_from(val.map_err(|e| {
                            Error::AvroError("couldn't read Avro value".to_owned(), e)
                        })?)
                    })
                    .collect::<Result<Vec<P>, _>>()?
            };

        Ok((header, packets))
    }
}

/// Allows writing files, including signature file construction, from an
/// ingestion or validation batch containing a header, a packet file and a
/// signature.
pub struct BatchWriter<'a, H: Header, P: Packet> {
    batch: Batch<H, P>,
    transport: &'a dyn Transport,
    trace_id: &'a Uuid,
}

impl<'a, H: Header, P: Packet> BatchWriter<'a, H, P> {
    pub fn new(batch: Batch<H, P>, transport: &'a dyn Transport, trace_id: &'a Uuid) -> Self {
        BatchWriter {
            batch,
            transport,
            trace_id,
        }
    }

    pub fn path(&self) -> String {
        self.transport.path()
    }

    /// Encode the provided header into Avro, sign that representation with the
    /// provided key and write the header into the batch. Returns the signature
    /// on success.
    pub fn put_header(&self, header: &H, key: &EcdsaKeyPair) -> Result<Signature> {
        let mut transport_writer = self.transport.put(self.batch.header_key(), self.trace_id)?;
        let mut header_buf = Vec::new();
        header.write(&mut header_buf)?;

        transport_writer.write_all(&header_buf)?;
        transport_writer
            .complete_upload()
            .context("failed to complete batch header upload")?;

        // TODO(brandon): per the advice of ring's doc comments, create a single SystemRandom &
        // reuse it throughout the application rather than creating a new SystemRandom each time.
        key.sign(&SystemRandom::new(), &header_buf)
            .context("failed to sign header file")
    }

    /// Creates an avro_rs::Writer and provides it to the caller-provided
    /// function, which may then write arbitrarily many packets into it. The
    /// Avro encoding of the packet will be digested while they are written.
    /// The operation should return Ok(()) when it has finished successfully or
    /// some Err() otherwise. packet_file_writer returns the digest of all the
    /// content written by the operation.
    pub fn packet_file_writer<F>(&self, operation: F) -> Result<Digest>
    where
        F: FnOnce(&mut Writer<&mut DigestWriter<&mut Box<dyn TransportWriter>>>) -> Result<()>,
    {
        let mut transport_writer = self
            .transport
            .put(self.batch.packet_file_key(), self.trace_id)?;
        let mut digest_writer = DigestWriter::new(&mut transport_writer);
        let mut writer = Writer::new(P::schema(), &mut digest_writer);

        let result = operation(&mut writer);
        writer
            .flush()
            .with_context(|| format!("failed to flush Avro writer ({:?})", result))?;

        if let Err(e) = result {
            transport_writer
                .cancel_upload()
                .with_context(|| format!("Encountered while handling: {}", e))?;
            return Err(e);
        }
        let digest = digest_writer.finish();
        transport_writer
            .complete_upload()
            .context("failed to complete packet file upload")?;
        Ok(digest)
    }

    /// Constructs a signature structure from the provided buffers and writes it
    /// to the batch's signature file
    pub fn put_signature(&self, signature: &Signature, key_identifier: &str) -> Result<()> {
        let batch_signature = BatchSignature {
            batch_header_signature: signature.as_ref().to_vec(),
            key_identifier: key_identifier.to_string(),
        };
        let mut writer = self
            .transport
            .put(self.batch.signature_key(), self.trace_id)?;
        batch_signature
            .write(&mut writer)
            .context("failed to write signature")?;
        writer
            .complete_upload()
            .context("failed to complete signature upload")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        idl::{IngestionDataSharePacket, IngestionHeader},
        logging::setup_test_logging,
        test_utils::{
            default_facilitator_signing_public_key, default_ingestor_private_key,
            default_ingestor_public_key, DEFAULT_TRACE_ID,
        },
        transport::LocalFileTransport,
    };
    use lazy_static::lazy_static;
    use std::fmt::Debug;

    const INSTANCE_NAME: &str = "fake-instance";
    const AGGREGATION_NAME: &str = "fake-aggregation";
    const BATCH_ID: Uuid = Uuid::from_bytes([0; 16]);

    lazy_static! {
        static ref START_TIMESTAMP: NaiveDateTime = NaiveDateTime::from_timestamp(1234567890, 654321);
        static ref END_TIMESTAMP: NaiveDateTime = NaiveDateTime::from_timestamp(2234567890, 654321);

        static ref INGESTION_HEADER: IngestionHeader = IngestionHeader {
            batch_uuid: BATCH_ID,
            name: AGGREGATION_NAME.to_owned(),
            bins: 2,
            epsilon: 1.601,
            prime: 17,
            number_of_servers: 2,
            hamming_weight: None,
            batch_start_time: 789456123,
            batch_end_time: 789456321,
            packet_file_digest: Vec::new(),
        };
        static ref INGESTION_DATA_SHARE_PACKETS: Vec<IngestionDataSharePacket> = vec![
            IngestionDataSharePacket {
                uuid: Uuid::from_bytes([1; 16]),
                encrypted_payload: vec![0u8, 1u8, 2u8, 3u8],
                encryption_key_id: Some("fake-key-1".to_owned()),
                r_pit: 1,
                version_configuration: Some("config-1".to_owned()),
                device_nonce: None,
            },
            IngestionDataSharePacket {
                uuid: Uuid::from_bytes([2; 16]),
                encrypted_payload: vec![4u8, 5u8, 6u8, 7u8],
                encryption_key_id: None,
                r_pit: 2,
                version_configuration: None,
                device_nonce: Some(vec![8u8, 9u8, 10u8, 11u8]),
            },
            IngestionDataSharePacket {
                uuid: Uuid::from_bytes([2; 16]),
                encrypted_payload: vec![8u8, 9u8, 10u8, 11u8],
                encryption_key_id: Some("fake-key-3".to_owned()),
                r_pit: 3,
                version_configuration: None,
                device_nonce: None,
            },
        ];

        static ref VALIDATION_HEADER: ValidationHeader = ValidationHeader {
            batch_uuid: BATCH_ID,
            name: AGGREGATION_NAME.to_owned(),
            bins: 2,
            epsilon: 1.601,
            prime: 17,
            number_of_servers: 2,
            hamming_weight: Some(12),
            packet_file_digest: Vec::new(), // set by test code
        };
        static ref VALIDATION_PACKETS: Vec<ValidationPacket> = vec![
            ValidationPacket {
                uuid: Uuid::from_bytes([3; 16]),
                f_r: 1,
                g_r: 2,
                h_r: 3,
            },
            ValidationPacket {
                uuid: Uuid::from_bytes([4; 16]),
                f_r: 4,
                g_r: 5,
                h_r: 6,
            },
            ValidationPacket {
                uuid: Uuid::from_bytes([5; 16]),
                f_r: 7,
                g_r: 8,
                h_r: 9,
            },
        ];

        static ref SUM_PART: SumPart = SumPart {
            batch_uuids: vec![BATCH_ID],
            name: AGGREGATION_NAME.to_owned(),
            bins: 2,
            epsilon: 1.601,
            prime: 17,
            number_of_servers: 2,
            hamming_weight: Some(12),
            sum: vec![12, 13, 14],
            aggregation_start_time: 789456123,
            aggregation_end_time: 789456321,
            packet_file_digest: Vec::new(), // set by test code
            total_individual_clients: 2,
        };
        static ref INVALID_PACKETS: Vec<InvalidPacket> = vec![
            InvalidPacket {
                uuid: Uuid::from_bytes([6; 16]),
            },
            InvalidPacket {
                uuid: Uuid::from_bytes([7; 16]),
            },
            InvalidPacket {
                uuid: Uuid::from_bytes([8; 16]),
            },
        ];
    }

    #[allow(clippy::too_many_arguments)] // Grandfathered in
    fn roundtrip_batch<'a, H, P>(
        header: &H,
        packets: &[P],
        base_path: String,
        filenames: &[String],
        batch_writer: &BatchWriter<'a, H, P>,
        batch_reader: &BatchReader<'a, H, P>,
        transport: &LocalFileTransport,
        write_key: &EcdsaKeyPair,
        read_key: &UnparsedPublicKey<Vec<u8>>,
        keys_match: bool,
    ) where
        H: Header + DigestSetter + Clone + PartialEq + Debug,
        P: Packet + PartialEq + Debug,
    {
        let packet_file_digest = batch_writer
            .packet_file_writer(|mut packet_writer| {
                for packet in packets {
                    packet.write(&mut packet_writer)?;
                }
                Ok(())
            })
            .expect("failed to write packets");

        let mut header = header.clone();
        header.set_packet_file_digest(packet_file_digest.as_ref().to_vec());

        let header_signature = batch_writer
            .put_header(&header, write_key)
            .expect("failed to write header");

        let res = batch_writer.put_signature(&header_signature, "key-identifier");
        assert!(res.is_ok(), "failed to put signature: {:?}", res.err());

        // Verify file layout is as expected
        for extension in filenames {
            transport
                .get(&format!("{}.{}", base_path, extension), &DEFAULT_TRACE_ID)
                .unwrap_or_else(|_| panic!("could not get batch file {}", extension));
        }

        let mut key_map = HashMap::new();
        key_map.insert("key-identifier".to_owned(), read_key.clone());
        let read_result = batch_reader.read(&key_map);
        if !keys_match {
            assert!(
                read_result.is_err(),
                "read should fail with mismatched keys"
            );
            return;
        }
        assert!(
            read_result.is_ok(),
            "failed to read header from batch {:?}",
            read_result.err()
        );
        let (header_again, packets_again) = read_result.unwrap();

        assert_eq!(header, header_again, "header does not match");

        let mut packets_again_iter = packets_again.iter();
        for packet in packets {
            let packet_again = packets_again_iter.next().expect("no packets remaining");
            assert_eq!(packet, packet_again, "packet does not match");
        }

        // One more read should get EOF
        assert!(packets_again_iter.next().is_none());
    }

    #[test]
    fn roundtrip_ingestion_batch_ok() {
        roundtrip_ingestion_batch(true)
    }

    #[test]
    fn roundtrip_ingestion_batch_bad_read_key() {
        roundtrip_ingestion_batch(false)
    }

    fn roundtrip_ingestion_batch(keys_match: bool) {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let batch_writer = BatchWriter::new(
            Batch::new_ingestion(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP),
            &write_transport,
            &DEFAULT_TRACE_ID,
        );
        let batch_reader = BatchReader::new(
            Batch::new_ingestion(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP),
            &read_transport,
            false,
            &DEFAULT_TRACE_ID,
            &logger,
        );
        let base_path = format!(
            "{}/{}/{}",
            AGGREGATION_NAME,
            END_TIMESTAMP.format(DATE_FORMAT),
            BATCH_ID.to_hyphenated()
        );
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            &*INGESTION_HEADER,
            &*INGESTION_DATA_SHARE_PACKETS,
            base_path,
            &[
                "batch".to_owned(),
                "batch.avro".to_owned(),
                "batch.sig".to_owned(),
            ],
            &batch_writer,
            &batch_reader,
            &verify_transport,
            &default_ingestor_private_key().key,
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn roundtrip_validation_batch_first_ok() {
        roundtrip_validation_batch(true, true)
    }

    #[test]
    fn roundtrip_validation_batch_first_bad_read_key() {
        roundtrip_validation_batch(true, false)
    }

    #[test]
    fn roundtrip_validation_batch_second_ok() {
        roundtrip_validation_batch(false, true)
    }

    #[test]
    fn roundtrip_validation_batch_second_bad_read_key() {
        roundtrip_validation_batch(false, false)
    }

    fn roundtrip_validation_batch(is_first: bool, keys_match: bool) {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let batch_writer = BatchWriter::new(
            Batch::new_validation(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP, is_first),
            &write_transport,
            &DEFAULT_TRACE_ID,
        );
        let batch_reader = BatchReader::new(
            Batch::new_validation(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP, is_first),
            &read_transport,
            false,
            &DEFAULT_TRACE_ID,
            &logger,
        );
        let base_path = format!(
            "{}/{}/{}",
            AGGREGATION_NAME,
            END_TIMESTAMP.format(DATE_FORMAT),
            BATCH_ID.to_hyphenated()
        );
        let filenames = if is_first {
            [
                "validity_0".to_owned(),
                "validity_0.avro".to_owned(),
                "validity_0.sig".to_owned(),
            ]
        } else {
            [
                "validity_1".to_owned(),
                "validity_1.avro".to_owned(),
                "validity_1.sig".to_owned(),
            ]
        };
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            &*VALIDATION_HEADER,
            &*VALIDATION_PACKETS,
            base_path,
            &filenames,
            &batch_writer,
            &batch_reader,
            &verify_transport,
            &default_ingestor_private_key().key,
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn roundtrip_sum_batch_first_ok() {
        roundtrip_sum_batch(true, true)
    }

    #[test]
    fn roundtrip_sum_batch_first_bad_read_key() {
        roundtrip_sum_batch(true, false)
    }

    #[test]
    fn roundtrip_sum_batch_second_ok() {
        roundtrip_sum_batch(false, true)
    }

    #[test]
    fn roundtrip_sum_batch_second_bad_read_key() {
        roundtrip_sum_batch(false, false)
    }

    fn roundtrip_sum_batch(is_first: bool, keys_match: bool) {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let batch_writer = BatchWriter::new(
            Batch::new_sum(
                INSTANCE_NAME,
                AGGREGATION_NAME,
                &START_TIMESTAMP,
                &END_TIMESTAMP,
                is_first,
            ),
            &write_transport,
            &DEFAULT_TRACE_ID,
        );
        let batch_reader = BatchReader::new(
            Batch::new_sum(
                INSTANCE_NAME,
                AGGREGATION_NAME,
                &START_TIMESTAMP,
                &END_TIMESTAMP,
                is_first,
            ),
            &read_transport,
            false,
            &DEFAULT_TRACE_ID,
            &logger,
        );
        let batch_path = format!(
            "{}/{}/{}-{}",
            INSTANCE_NAME,
            AGGREGATION_NAME,
            START_TIMESTAMP.format(AGGREGATION_DATE_FORMAT),
            END_TIMESTAMP.format(AGGREGATION_DATE_FORMAT)
        );
        let filenames = if is_first {
            [
                "sum_0".to_owned(),
                "invalid_uuid_0.avro".to_owned(),
                "sum_0.sig".to_owned(),
            ]
        } else {
            [
                "sum_1".to_owned(),
                "invalid_uuid_1.avro".to_owned(),
                "sum_1.sig".to_owned(),
            ]
        };
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            &*SUM_PART,
            &*INVALID_PACKETS,
            batch_path,
            &filenames,
            &batch_writer,
            &batch_reader,
            &verify_transport,
            &default_ingestor_private_key().key,
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn permit_malformed_batch() {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let batch_writer = BatchWriter::new(
            Batch::new_ingestion(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP),
            &write_transport,
            &DEFAULT_TRACE_ID,
        );
        let mut batch_reader = BatchReader::new(
            Batch::new_ingestion(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP),
            &read_transport,
            true, // permit_malformed_batch
            &DEFAULT_TRACE_ID,
            &logger,
        );

        let metrics_collector = BatchReaderMetricsCollector::new("test").unwrap();
        batch_reader.set_metrics_collector(&metrics_collector);

        let packet = IngestionDataSharePacket {
            uuid: Uuid::new_v4(),
            encrypted_payload: vec![0u8, 1u8, 2u8, 3u8],
            encryption_key_id: Some("fake-key-1".to_owned()),
            r_pit: 1,
            version_configuration: Some("config-1".to_owned()),
            device_nonce: None,
        };

        batch_writer
            .packet_file_writer(|mut packet_writer| {
                packet.write(&mut packet_writer)?;
                Ok(())
            })
            .expect("failed to write packets");

        let header = IngestionHeader {
            batch_uuid: BATCH_ID,
            name: AGGREGATION_NAME.to_string(),
            bins: 2,
            epsilon: 1.601,
            prime: 17,
            number_of_servers: 2,
            hamming_weight: None,
            batch_start_time: 789456123,
            batch_end_time: 789456321,
            // Use bogus packet file digest
            packet_file_digest: vec![0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8],
        };

        let header_signature = batch_writer
            .put_header(&header, &default_ingestor_private_key().key)
            .expect("failed to write header");

        let res = batch_writer.put_signature(&header_signature, "key-identifier");
        assert!(res.is_ok(), "failed to put signature: {:?}", res.err());

        // Verify with different key than we signed with
        let mut key_map = HashMap::new();
        key_map.insert(
            "key-identifier".to_owned(),
            default_facilitator_signing_public_key(),
        );

        assert_eq!(
            metrics_collector
                .invalid_validation_batches
                .with_label_values(&["header"])
                .get(),
            0
        );

        assert_eq!(
            metrics_collector
                .invalid_validation_batches
                .with_label_values(&["packet_file"])
                .get(),
            0
        );

        batch_reader.read(&key_map).unwrap();

        assert_eq!(
            metrics_collector
                .invalid_validation_batches
                .with_label_values(&["header"])
                .get(),
            1
        );

        assert_eq!(
            metrics_collector
                .invalid_validation_batches
                .with_label_values(&["packet_file"])
                .get(),
            1
        );
    }

    trait DigestSetter {
        fn set_packet_file_digest(&mut self, digest: Vec<u8>);
    }

    impl DigestSetter for IngestionHeader {
        fn set_packet_file_digest(&mut self, digest: Vec<u8>) {
            self.packet_file_digest = digest;
        }
    }

    impl DigestSetter for ValidationHeader {
        fn set_packet_file_digest(&mut self, digest: Vec<u8>) {
            self.packet_file_digest = digest;
        }
    }

    impl DigestSetter for SumPart {
        fn set_packet_file_digest(&mut self, digest: Vec<u8>) {
            self.packet_file_digest = digest;
        }
    }
}
