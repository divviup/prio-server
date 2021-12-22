use crate::{
    idl::{BatchSignature, Header, Packet},
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
pub struct Batch {
    header_path: String,
    signature_path: String,
    packet_file_path: String,
}

impl Batch {
    /// Creates a Batch representing an ingestion batch
    pub fn new_ingestion(aggregation_name: &str, batch_id: &Uuid, date: &NaiveDateTime) -> Batch {
        Batch::new(aggregation_name, batch_id, date, "batch")
    }

    /// Creates a Batch representing a validation batch
    pub fn new_validation(
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        is_first: bool,
    ) -> Batch {
        Batch::new(
            aggregation_name,
            batch_id,
            date,
            &format!("validity_{}", if is_first { 0 } else { 1 }),
        )
    }

    // Creates a batch representing a sum part batch
    pub fn new_sum(
        instance_name: &str,
        aggregation_name: &str,
        aggregation_start: &NaiveDateTime,
        aggregation_end: &NaiveDateTime,
        is_first: bool,
    ) -> Batch {
        let batch_path = format!(
            "{}/{}/{}-{}",
            instance_name,
            aggregation_name,
            aggregation_start.format(AGGREGATION_DATE_FORMAT),
            aggregation_end.format(AGGREGATION_DATE_FORMAT)
        );
        let filename = format!("sum_{}", if is_first { 0 } else { 1 });

        Batch {
            header_path: format!("{}.{}", batch_path, filename),
            signature_path: format!("{}.{}.sig", batch_path, filename),
            packet_file_path: format!(
                "{}.invalid_uuid_{}.avro",
                batch_path,
                if is_first { 0 } else { 1 }
            ),
        }
    }

    fn new(aggregation_name: &str, batch_id: &Uuid, date: &NaiveDateTime, filename: &str) -> Batch {
        let batch_path = format!(
            "{}/{}/{}",
            aggregation_name,
            date.format(DATE_FORMAT),
            batch_id.to_hyphenated()
        );
        Batch {
            header_path: format!("{}.{}", batch_path, filename),
            signature_path: format!("{}.{}.sig", batch_path, filename),
            packet_file_path: format!("{}.{}.avro", batch_path, filename),
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
pub struct BatchReader<'a, H, P> {
    trace_id: &'a Uuid,
    batch: Batch,
    transport: &'a mut dyn Transport,
    permit_malformed_batch: bool,
    metrics_collector: Option<&'a BatchReaderMetricsCollector>,
    logger: Logger,

    // These next two fields are not real and are used because not using H and P
    // in the struct definition is an error.
    phantom_header: PhantomData<*const H>,
    phantom_packet: PhantomData<*const P>,
}

impl<'a, H: Header, P: Packet> BatchReader<'a, H, P> {
    /// Create a new BatchReader which will fetch the specified batch from the
    /// provided transport. If permissive is true, then invalid signatures or
    /// packet file digest mismatches will be ignored.
    pub fn new(
        batch: Batch,
        transport: &'a mut dyn Transport,
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
            phantom_header: PhantomData,
            phantom_packet: PhantomData,
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
        &mut self,
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
pub struct BatchWriter<'a, H, P> {
    batch: Batch,
    transport: &'a mut dyn Transport,
    trace_id: &'a Uuid,
    phantom_header: PhantomData<*const H>,
    phantom_packet: PhantomData<*const P>,
}

impl<'a, H: Header, P: Packet> BatchWriter<'a, H, P> {
    pub fn new(batch: Batch, transport: &'a mut dyn Transport, trace_id: &'a Uuid) -> Self {
        BatchWriter {
            batch,
            transport,
            trace_id,
            phantom_header: PhantomData,
            phantom_packet: PhantomData,
        }
    }

    pub fn path(&self) -> String {
        self.transport.path()
    }

    /// Encode the provided header into Avro, sign that representation with the
    /// provided key and write the header into the batch. Returns the signature
    /// on success.
    pub fn put_header(&mut self, header: &H, key: &EcdsaKeyPair) -> Result<Signature> {
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
    pub fn packet_file_writer<F>(&mut self, operation: F) -> Result<Digest>
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
    pub fn put_signature(&mut self, signature: &Signature, key_identifier: &str) -> Result<()> {
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

    #[allow(clippy::too_many_arguments)] // Grandfathered in
    fn roundtrip_batch<'a>(
        aggregation_name: String,
        batch_id: Uuid,
        base_path: String,
        filenames: &[String],
        batch_writer: &mut BatchWriter<'a, IngestionHeader, IngestionDataSharePacket>,
        batch_reader: &mut BatchReader<'a, IngestionHeader, IngestionDataSharePacket>,
        transport: &mut LocalFileTransport,
        write_key: &EcdsaKeyPair,
        read_key: &UnparsedPublicKey<Vec<u8>>,
        keys_match: bool,
    ) {
        // These tests cheat by using IngestionHeader and
        // IngestionDataSharePacket regardless of what kind of batch it is, but
        // since BatchReader and BatchWriter are generic in H and P, this
        // exercises what we care about in this module while saving us a bit of
        // typing.
        let packets = &[
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![0u8, 1u8, 2u8, 3u8],
                encryption_key_id: Some("fake-key-1".to_owned()),
                r_pit: 1,
                version_configuration: Some("config-1".to_owned()),
                device_nonce: None,
            },
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![4u8, 5u8, 6u8, 7u8],
                encryption_key_id: None,
                r_pit: 2,
                version_configuration: None,
                device_nonce: Some(vec![8u8, 9u8, 10u8, 11u8]),
            },
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![8u8, 9u8, 10u8, 11u8],
                encryption_key_id: Some("fake-key-3".to_owned()),
                r_pit: 3,
                version_configuration: None,
                device_nonce: None,
            },
        ];

        let packet_file_digest = batch_writer
            .packet_file_writer(|mut packet_writer| {
                packets[0].write(&mut packet_writer)?;
                packets[1].write(&mut packet_writer)?;
                packets[2].write(&mut packet_writer)?;
                Ok(())
            })
            .expect("failed to write packets");

        let header = IngestionHeader {
            batch_uuid: batch_id,
            name: aggregation_name,
            bins: 2,
            epsilon: 1.601,
            prime: 17,
            number_of_servers: 2,
            hamming_weight: None,
            batch_start_time: 789456123,
            batch_end_time: 789456321,
            packet_file_digest: packet_file_digest.as_ref().to_vec(),
        };

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
        let mut write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let aggregation_name = "fake-aggregation";
        let batch_id = Uuid::new_v4();
        let date = NaiveDateTime::from_timestamp(2234567890, 654321);

        let mut batch_writer: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchWriter::new(
                Batch::new_ingestion(aggregation_name, &batch_id, &date),
                &mut write_transport,
                &DEFAULT_TRACE_ID,
            );
        let mut batch_reader: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_ingestion(aggregation_name, &batch_id, &date),
                &mut read_transport,
                false,
                &DEFAULT_TRACE_ID,
                &logger,
            );
        let base_path = format!(
            "{}/{}/{}",
            aggregation_name,
            date.format(DATE_FORMAT),
            batch_id.to_hyphenated()
        );
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            aggregation_name.to_string(),
            batch_id,
            base_path,
            &[
                "batch".to_owned(),
                "batch.avro".to_owned(),
                "batch.sig".to_owned(),
            ],
            &mut batch_writer,
            &mut batch_reader,
            &mut verify_transport,
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
        let mut write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let aggregation_name = "fake-aggregation";
        let batch_id = Uuid::new_v4();
        let date = NaiveDateTime::from_timestamp(2234567890, 654321);

        let mut batch_writer: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchWriter::new(
                Batch::new_validation(aggregation_name, &batch_id, &date, is_first),
                &mut write_transport,
                &DEFAULT_TRACE_ID,
            );
        let mut batch_reader: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_validation(aggregation_name, &batch_id, &date, is_first),
                &mut read_transport,
                false,
                &DEFAULT_TRACE_ID,
                &logger,
            );
        let base_path = format!(
            "{}/{}/{}",
            aggregation_name,
            date.format(DATE_FORMAT),
            batch_id.to_hyphenated()
        );
        let first_filenames = &[
            "validity_0".to_owned(),
            "validity_0.avro".to_owned(),
            "validity_0.sig".to_owned(),
        ];
        let second_filenames = &[
            "validity_1".to_owned(),
            "validity_1.avro".to_owned(),
            "validity_1.sig".to_owned(),
        ];
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            aggregation_name.to_string(),
            batch_id,
            base_path,
            if is_first {
                first_filenames
            } else {
                second_filenames
            },
            &mut batch_writer,
            &mut batch_reader,
            &mut verify_transport,
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
        let mut write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let instance_name = "fake-instance";
        let aggregation_name = "fake-aggregation";
        let batch_id = Uuid::new_v4();
        let start = NaiveDateTime::from_timestamp(1234567890, 654321);
        let end = NaiveDateTime::from_timestamp(2234567890, 654321);

        let mut batch_writer: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchWriter::new(
                Batch::new_sum(instance_name, aggregation_name, &start, &end, is_first),
                &mut write_transport,
                &DEFAULT_TRACE_ID,
            );
        let mut batch_reader: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_sum(instance_name, aggregation_name, &start, &end, is_first),
                &mut read_transport,
                false,
                &DEFAULT_TRACE_ID,
                &logger,
            );
        let batch_path = format!(
            "{}/{}/{}-{}",
            instance_name,
            aggregation_name,
            start.format(AGGREGATION_DATE_FORMAT),
            end.format(AGGREGATION_DATE_FORMAT)
        );
        let first_filenames = &[
            "sum_0".to_owned(),
            "invalid_uuid_0.avro".to_owned(),
            "sum_0.sig".to_owned(),
        ];
        let second_filenames = &[
            "sum_1".to_owned(),
            "invalid_uuid_1.avro".to_owned(),
            "sum_1.sig".to_owned(),
        ];
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            aggregation_name.to_string(),
            batch_id,
            batch_path,
            if is_first {
                first_filenames
            } else {
                second_filenames
            },
            &mut batch_writer,
            &mut batch_reader,
            &mut verify_transport,
            &default_ingestor_private_key().key,
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn permit_malformed_batch() {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let mut write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let instance_name = "fake-instance";
        let aggregation_name = "fake-aggregation";
        let batch_id = Uuid::new_v4();
        let start = NaiveDateTime::from_timestamp(1234567890, 654321);
        let end = NaiveDateTime::from_timestamp(2234567890, 654321);

        let mut batch_writer: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchWriter::new(
                Batch::new_sum(instance_name, aggregation_name, &start, &end, true),
                &mut write_transport,
                &DEFAULT_TRACE_ID,
            );
        let mut batch_reader: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_sum(instance_name, aggregation_name, &start, &end, true),
                &mut read_transport,
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
            batch_uuid: batch_id,
            name: aggregation_name.to_string(),
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
}
