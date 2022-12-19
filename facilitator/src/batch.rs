use crate::{
    idl::{
        BatchSignature, Header, IdlError, IngestionDataSharePacket, IngestionHeader, InvalidPacket,
        Packet, SumPart, ValidationHeader, ValidationPacket,
    },
    metrics::BatchReaderMetricsCollector,
    transport::{Transport, TransportError, TransportWriter},
    BatchSigningKey, DigestWriter, ErrorClassification, DATE_FORMAT,
};
use avro_rs::{Reader, Writer};
use chrono::NaiveDateTime;
use ring::{
    digest::{digest, SHA256},
    rand::SystemRandom,
    signature::UnparsedPublicKey,
};
use slog::{o, warn, Logger};
use std::{
    collections::HashMap,
    fmt::Display,
    io::{self, Cursor, Read},
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
    phantom_header: PhantomData<H>,
    phantom_packet: PhantomData<P>,
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
            &format!("validity_{}", i32::from(!is_first)),
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
        let filename = format!("sum_{}", i32::from(!is_first));

        Self {
            header_path: format!("{}.{}", batch_path, filename),
            signature_path: format!("{}.{}.sig", batch_path, filename),
            packet_file_path: format!("{}.invalid_uuid_{}.avro", batch_path, i32::from(!is_first)),

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

/// Identifies which batch file an error is associated with.
#[derive(Debug)]
pub enum BatchFileKind {
    Packet,
    Header,
    Signature,
}

impl Display for BatchFileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self {
            BatchFileKind::Packet => write!(f, "packet"),
            BatchFileKind::Header => write!(f, "header"),
            BatchFileKind::Signature => write!(f, "signature"),
        }
    }
}

/// Errors that may occur when reading a batch.
#[derive(Debug, thiserror::Error)]
pub enum BatchReadError {
    #[error("failed to get {1} file from transport: {0}")]
    Transport(Box<TransportError>, BatchFileKind),
    #[error("failed to read {1} file from transport: {0}")]
    Io(io::Error, BatchFileKind),
    #[error(transparent)]
    Idl(#[from] IdlError),
    #[error("invalid signature on header with key {0}")]
    InvalidSignature(String),
    #[error("key identifier {0} not present in key map {1:?}")]
    UnknownKeyIdentifier(String, Vec<String>),
    #[error("packet file digest in header {0} does not match actual packet file digest {1}")]
    DigestMismatch(String, String),
}

impl ErrorClassification for BatchReadError {
    fn is_retryable(&self) -> bool {
        match self {
            // For errors that are straightforwardly I/O issues, retry them later.
            BatchReadError::Transport(_, _) | BatchReadError::Io(_, _) => true,
            // Bad signatures or digests cannot be resolved with retries.
            BatchReadError::InvalidSignature(_) | BatchReadError::DigestMismatch(_, _) => false,
            // If the key identifier is not recognized, that could be due to either an issue
            // with the submitted data or a serious configuration issue.
            BatchReadError::UnknownKeyIdentifier(_, _) => false,
            // Dispatch to wrapped error.
            BatchReadError::Idl(e) => e.is_retryable(),
        }
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
    ) -> Result<(H, Vec<P>), BatchReadError> {
        // Read the batch signature.
        let signature = BatchSignature::read(
            self.transport
                .get(self.batch.signature_key(), self.trace_id)
                .map_err(|e| BatchReadError::Transport(Box::new(e), BatchFileKind::Signature))?,
        )?;

        // Read the header & packets, if not included in the signature.
        let (header_bytes, packet_bytes) =
            if signature.batch_header_bytes.is_some() && signature.packet_bytes.is_some() {
                (
                    signature.batch_header_bytes.unwrap(),
                    signature.packet_bytes.unwrap(),
                )
            } else {
                let mut header_bytes = Vec::new();
                self.transport
                    .get(self.batch.header_key(), self.trace_id)
                    .map_err(|e| BatchReadError::Transport(Box::new(e), BatchFileKind::Header))?
                    .read_to_end(&mut header_bytes)
                    .map_err(|e| BatchReadError::Io(e, BatchFileKind::Header))?;

                // We read all of the packets, rather than streaming bytes as we
                // need them to yield a packet, because we need to check that the
                // header's recorded digest matches the actual content's digest. The
                // ingestion server authors suggest that the maximum batch size will be
                // ~400MiB, so this should work out okay.
                let mut packet_bytes = Vec::new();
                let packet_get_result = self
                    .transport
                    .get(self.batch.packet_file_key(), self.trace_id);

                match packet_get_result {
                    Ok(mut reader) => {
                        reader
                            .read_to_end(&mut packet_bytes)
                            .map_err(|e| BatchReadError::Io(e, BatchFileKind::Packet))?;
                    }
                    Err(ref err) => {
                        // We treat ObjectNotFoundErrors as "empty" packet
                        // objects, as our transport writers do not create
                        // 0-byte objects at all. This is a bug in the
                        // in the transport writer, but at the point this was
                        // fixed this behavior was long-standing and visible to
                        // external partners, so it's better to simply accept
                        // missing objects as representing a 0-byte packet
                        // file. We check that the packet file _should_ be
                        // empty when we check the digest below, so this won't
                        // let someone get away with something sneaky by
                        // deleting a packet file.
                        if !matches!(&err, TransportError::ObjectNotFoundError(_, _)) {
                            packet_get_result.map_err(|e| {
                                BatchReadError::Transport(Box::new(e), BatchFileKind::Packet)
                            })?;
                        };
                    }
                }

                (header_bytes, packet_bytes)
            };

        // Check the header's signature, and parse it if the signature is valid.
        let key_identifier = signature.key_identifier;
        let public_key = public_keys.get(&key_identifier).ok_or_else(|| {
            BatchReadError::UnknownKeyIdentifier(
                key_identifier.clone(),
                public_keys.keys().cloned().collect(),
            )
        })?;
        if public_key
            .verify(&header_bytes, &signature.batch_header_signature)
            .is_err()
        {
            if let Some(collector) = self.metrics_collector {
                collector
                    .invalid_validation_batches
                    .with_label_values(&["header"])
                    .inc();
            }
            let signature_error = BatchReadError::InvalidSignature(key_identifier);
            if self.permit_malformed_batch {
                warn!(self.logger, "{}", signature_error);
            } else {
                return Err(signature_error);
            }
        }
        let header = H::read(Cursor::new(header_bytes))?;

        // Verify that the packet digest matches header's recorded digest.
        let packet_digest = digest(&SHA256, &packet_bytes);
        if header.packet_file_digest() != packet_digest.as_ref() {
            if let Some(collector) = self.metrics_collector {
                collector
                    .invalid_validation_batches
                    .with_label_values(&["packet_file"])
                    .inc();
            }
            let digest_error = BatchReadError::DigestMismatch(
                hex::encode(header.packet_file_digest()),
                hex::encode(packet_digest),
            );
            if self.permit_malformed_batch {
                warn!(self.logger, "{}", digest_error);
            } else {
                return Err(digest_error);
            }
        }

        // Parse packets.
        // avro_rs readers will always attempt to read a header on creation;
        // apparently, avro_rs writers don't bother writing a header unless at
        // least one record is written. So we need to special-case zero bytes
        // of content as returning an empty vector of packets, because trying
        // to use a Reader will error out.
        let packets = if packet_bytes.is_empty() {
            Vec::new()
        } else {
            Reader::with_schema(P::schema(), Cursor::new(&packet_bytes))
                .map_err(|e| IdlError::Avro(e, "reading avro header"))?
                .map(|res| {
                    let val = res.map_err(|e| IdlError::Avro(e, "reading record"))?;
                    P::try_from(val)
                })
                .collect::<Result<Vec<P>, _>>()?
        };

        Ok((header, packets))
    }
}

/// Errors that may occur when writing a batch.
#[derive(Debug, thiserror::Error)]
pub enum BatchWriteError {
    #[error("couldn't flush packet writer: {0}")]
    Flush(avro_rs::Error),
    #[error("couldn't write {1}: {0}")]
    Idl(IdlError, BatchFileKind),
    #[error("couldn't write {1}: {0}")]
    Io(io::Error, BatchFileKind),
    #[error("couldn't sign header")]
    Signing,
    #[error("couldn't upload {1} file: {0}")]
    Upload(Box<TransportError>, BatchFileKind),
    #[error("couldn't complete {1} file upload: {0}")]
    CompleteUpload(Box<TransportError>, BatchFileKind),
}

impl ErrorClassification for BatchWriteError {
    fn is_retryable(&self) -> bool {
        match self {
            //Retry I/O errors, network errors, and cloud API errors.
            BatchWriteError::Flush(_)
            | BatchWriteError::Io(_, _)
            | BatchWriteError::Upload(_, _)
            | BatchWriteError::CompleteUpload(_, _) => true,
            // Dispatch to the wrapped error.
            BatchWriteError::Idl(e, _) => e.is_retryable(),
            // Signing errors could be due to getrandom failures.
            BatchWriteError::Signing => true,
        }
    }
}

/// Allows writing files, including signature file construction, from an
/// ingestion or validation batch containing a header, a packet file and a
/// signature.
pub struct BatchWriter<'a, H: Header, P: Packet> {
    batch: Batch<H, P>,
    transport: &'a dyn Transport,
    trace_id: &'a Uuid,
    use_bogus_packet_file_digest: bool,
    single_object_write: bool,
}

impl<'a, H: Header, P: Packet> BatchWriter<'a, H, P> {
    pub fn new(batch: Batch<H, P>, transport: &'a dyn Transport, trace_id: &'a Uuid) -> Self {
        BatchWriter {
            batch,
            transport,
            trace_id,
            use_bogus_packet_file_digest: false,
            single_object_write: false,
        }
    }

    pub fn path(&self) -> String {
        self.transport.path()
    }

    /// Sets whether this BatchWriter will use a bogus value for the packet
    /// file digest when constructing the header of a validation batch. This
    /// is intended only for testing.
    pub fn set_use_bogus_packet_file_digest(&mut self, bogus: bool) {
        self.use_bogus_packet_file_digest = bogus;
    }

    /// Sets whether this BatchWriter will write the batch into a single
    /// object. Otherwise, the batch writer will write into three separate
    /// objects (signature, header, & packet objects).
    pub fn set_use_single_object_write(&mut self, single_object_write: bool) {
        self.single_object_write = single_object_write;
    }

    pub fn write<I: IntoIterator<Item = P>>(
        &self,
        key: &BatchSigningKey,
        header: H,
        packets: I,
    ) -> Result<(), BatchWriteError> {
        if self.single_object_write {
            self.single_object_write(key, header, packets)
        } else {
            self.multi_object_write(key, header, packets)
        }
    }

    fn single_object_write<I: IntoIterator<Item = P>>(
        &self,
        key: &BatchSigningKey,
        mut header: H,
        packets: I,
    ) -> Result<(), BatchWriteError> {
        // Serialize packets in memory.
        let mut packet_bytes = Vec::new();
        let mut digest_writer = DigestWriter::new(&mut packet_bytes);
        let mut packet_writer = Writer::new(P::schema(), &mut digest_writer);
        for packet in packets {
            packet
                .write(&mut packet_writer)
                .map_err(|e| BatchWriteError::Idl(e, BatchFileKind::Packet))?;
        }
        packet_writer.flush().map_err(BatchWriteError::Flush)?;

        // If the caller requested it, we insert a bogus packet file digest into
        // the peer validaton batch headers instead of the real computed
        // digest. This is meant to simulate a buggy peer data share processor,
        // so that we can test how the aggregation step behaves.
        let packet_digest = if self.use_bogus_packet_file_digest {
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8]
        } else {
            digest_writer.finish().as_ref().to_vec()
        };

        // Serialize header in memory.
        header.set_packet_file_digest(packet_digest);
        let mut header_bytes = Vec::new();
        header
            .write(&mut header_bytes)
            .map_err(|e| BatchWriteError::Idl(e, BatchFileKind::Header))?;
        // TODO(brandon): per the advice of ring's doc comments, create a single SystemRandom &
        // reuse it throughout the application rather than creating a new SystemRandom each time.
        let header_signature = key
            .key
            .sign(&SystemRandom::new(), &header_bytes)
            .map_err(|_| BatchWriteError::Signing)?;

        // Write signature, including serialized header & packets.
        let batch_signature = BatchSignature {
            batch_header_signature: header_signature.as_ref().to_vec(),
            key_identifier: key.identifier.clone(),
            batch_header_bytes: Some(header_bytes),
            packet_bytes: Some(packet_bytes),
        };
        let mut transport_writer = self
            .transport
            .put(self.batch.signature_key(), self.trace_id)
            .map_err(|e| BatchWriteError::Upload(Box::new(e), BatchFileKind::Signature))?;
        batch_signature
            .write(&mut transport_writer)
            .map_err(|e| BatchWriteError::Idl(e, BatchFileKind::Signature))?;
        transport_writer
            .complete_upload()
            .map_err(|e| BatchWriteError::CompleteUpload(Box::new(e), BatchFileKind::Signature))
    }

    fn multi_object_write<I: IntoIterator<Item = P>>(
        &self,
        key: &BatchSigningKey,
        mut header: H,
        packets: I,
    ) -> Result<(), BatchWriteError> {
        // Write packets.
        let mut transport_writer = self
            .transport
            .put(self.batch.packet_file_key(), self.trace_id)
            .map_err(|e| BatchWriteError::Upload(Box::new(e), BatchFileKind::Packet))?;
        let mut digest_writer = DigestWriter::new(&mut transport_writer);
        let mut packet_writer = Writer::new(P::schema(), &mut digest_writer);

        for packet in packets {
            packet
                .write(&mut packet_writer)
                .map_err(|e| BatchWriteError::Idl(e, BatchFileKind::Packet))?;
        }

        packet_writer.flush().map_err(BatchWriteError::Flush)?;
        // If the caller requested it, we insert a bogus packet file digest into
        // the peer validaton batch headers instead of the real computed
        // digest. This is meant to simulate a buggy peer data share processor,
        // so that we can test how the aggregation step behaves.
        let packet_digest = if self.use_bogus_packet_file_digest {
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8]
        } else {
            digest_writer.finish().as_ref().to_vec()
        };
        transport_writer
            .complete_upload()
            .map_err(|e| BatchWriteError::CompleteUpload(Box::new(e), BatchFileKind::Packet))?;

        // Write header.
        // TODO(brandon): per the advice of ring's doc comments, create a single SystemRandom &
        // reuse it throughout the application rather than creating a new SystemRandom each time.
        header.set_packet_file_digest(packet_digest);
        let mut header_bytes = Vec::new();
        header
            .write(&mut header_bytes)
            .map_err(|e| BatchWriteError::Idl(e, BatchFileKind::Header))?;
        let header_signature = key
            .key
            .sign(&SystemRandom::new(), &header_bytes)
            .map_err(|_| BatchWriteError::Signing)?;

        let mut transport_writer = self
            .transport
            .put(self.batch.header_key(), self.trace_id)
            .map_err(|e| BatchWriteError::Upload(Box::new(e), BatchFileKind::Header))?;
        transport_writer
            .write_all(&header_bytes)
            .map_err(|e| BatchWriteError::Io(e, BatchFileKind::Header))?;
        transport_writer
            .complete_upload()
            .map_err(|e| BatchWriteError::CompleteUpload(Box::new(e), BatchFileKind::Header))?;

        // Write signature.
        let batch_signature = BatchSignature {
            batch_header_signature: header_signature.as_ref().to_vec(),
            key_identifier: key.identifier.clone(),
            batch_header_bytes: None,
            packet_bytes: None,
        };
        let mut transport_writer = self
            .transport
            .put(self.batch.signature_key(), self.trace_id)
            .map_err(|e| BatchWriteError::Upload(Box::new(e), BatchFileKind::Signature))?;
        batch_signature
            .write(&mut transport_writer)
            .map_err(|e| BatchWriteError::Idl(e, BatchFileKind::Signature))?;
        transport_writer
            .complete_upload()
            .map_err(|e| BatchWriteError::CompleteUpload(Box::new(e), BatchFileKind::Signature))?;

        Ok(())
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
        static ref START_TIMESTAMP: NaiveDateTime = NaiveDateTime::from_timestamp_opt(1234567890, 654321).unwrap();
        static ref END_TIMESTAMP: NaiveDateTime = NaiveDateTime::from_timestamp_opt(2234567890, 654321).unwrap();

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

    fn roundtrip_batch<'a, H, P>(
        header: &H,
        packets: &[P],
        base_path: String,
        filenames: &[String],
        batch_writer: &BatchWriter<'a, H, P>,
        batch_reader: &BatchReader<'a, H, P>,
        transport: &LocalFileTransport,
        write_key: &BatchSigningKey,
        read_key: &UnparsedPublicKey<Vec<u8>>,
        keys_match: bool,
    ) where
        H: Header + Clone + PartialEq + Debug,
        P: Packet + Clone + PartialEq + Debug,
    {
        batch_writer
            .write(write_key, header.clone(), packets.iter().map(Clone::clone))
            .expect("Couldn't write batch");

        // Verify file layout is as expected
        for extension in filenames {
            transport
                .get(&format!("{}.{}", base_path, extension), &DEFAULT_TRACE_ID)
                .unwrap_or_else(|_| panic!("could not get batch file {}", extension));
        }

        let mut key_map = HashMap::new();
        key_map.insert(write_key.identifier.clone(), read_key.clone());
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
        let (mut header_again, packets_again) = read_result.unwrap();

        // Clearing the header's packet digest is required to make the written
        // header match the given template; the digest itself is already
        // checked by BatchReader::read.
        header_again.set_packet_file_digest(Vec::new());
        assert_eq!(header, &header_again, "header does not match");

        let mut packets_again_iter = packets_again.iter();
        for packet in packets {
            let packet_again = packets_again_iter.next().expect("no packets remaining");
            assert_eq!(packet, packet_again, "packet does not match");
        }

        // One more read should get EOF
        assert!(packets_again_iter.next().is_none());
    }

    #[test]
    fn roundtrip_multi_object_ingestion_batch_ok() {
        roundtrip_ingestion_batch(true, false)
    }

    #[test]
    fn roundtrip_multi_object_ingestion_batch_bad_read_key() {
        roundtrip_ingestion_batch(false, false)
    }

    #[test]
    fn roundtrip_single_object_ingestion_batch_ok() {
        roundtrip_ingestion_batch(true, true)
    }

    #[test]
    fn roundtrip_single_object_ingestion_batch_bad_read_key() {
        roundtrip_ingestion_batch(false, true)
    }

    fn roundtrip_ingestion_batch(keys_match: bool, single_object_write: bool) {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let mut batch_writer = BatchWriter::new(
            Batch::new_ingestion(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP),
            &write_transport,
            &DEFAULT_TRACE_ID,
        );
        batch_writer.set_use_single_object_write(single_object_write);
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
        let filenames = if single_object_write {
            vec!["batch.sig".to_owned()]
        } else {
            vec![
                "batch".to_owned(),
                "batch.avro".to_owned(),
                "batch.sig".to_owned(),
            ]
        };
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            &*INGESTION_HEADER,
            &INGESTION_DATA_SHARE_PACKETS,
            base_path,
            &filenames,
            &batch_writer,
            &batch_reader,
            &verify_transport,
            &default_ingestor_private_key(),
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn roundtrip_multi_object_validation_batch_first_ok() {
        roundtrip_validation_batch(true, true, false)
    }

    #[test]
    fn roundtrip_multi_object_validation_batch_first_bad_read_key() {
        roundtrip_validation_batch(true, false, false)
    }

    #[test]
    fn roundtrip_multi_object_validation_batch_second_ok() {
        roundtrip_validation_batch(false, true, false)
    }

    #[test]
    fn roundtrip_multi_object_validation_batch_second_bad_read_key() {
        roundtrip_validation_batch(false, false, false)
    }

    #[test]
    fn roundtrip_single_object_validation_batch_first_ok() {
        roundtrip_validation_batch(true, true, true)
    }

    #[test]
    fn roundtrip_single_object_validation_batch_first_bad_read_key() {
        roundtrip_validation_batch(true, false, true)
    }

    #[test]
    fn roundtrip_single_object_validation_batch_second_ok() {
        roundtrip_validation_batch(false, true, true)
    }

    #[test]
    fn roundtrip_single_object_validation_batch_second_bad_read_key() {
        roundtrip_validation_batch(false, false, true)
    }

    fn roundtrip_validation_batch(is_first: bool, keys_match: bool, single_object_write: bool) {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let mut batch_writer = BatchWriter::new(
            Batch::new_validation(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP, is_first),
            &write_transport,
            &DEFAULT_TRACE_ID,
        );
        batch_writer.set_use_single_object_write(single_object_write);
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
        let idx = i32::from(!is_first);
        let filenames = if single_object_write {
            vec![format!("validity_{}.sig", idx)]
        } else {
            vec![
                format!("validity_{}", idx),
                format!("validity_{}.avro", idx),
                format!("validity_{}.sig", idx),
            ]
        };
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            &*VALIDATION_HEADER,
            &VALIDATION_PACKETS,
            base_path,
            &filenames,
            &batch_writer,
            &batch_reader,
            &verify_transport,
            &default_ingestor_private_key(),
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn roundtrip_multi_object_sum_batch_first_ok() {
        roundtrip_sum_batch(true, true, false)
    }

    #[test]
    fn roundtrip_multi_object_sum_batch_first_bad_read_key() {
        roundtrip_sum_batch(true, false, false)
    }

    #[test]
    fn roundtrip_multi_object_sum_batch_second_ok() {
        roundtrip_sum_batch(false, true, false)
    }

    #[test]
    fn roundtrip_multi_object_sum_batch_second_bad_read_key() {
        roundtrip_sum_batch(false, false, false)
    }

    #[test]
    fn roundtrip_single_object_sum_batch_first_ok() {
        roundtrip_sum_batch(true, true, true)
    }

    #[test]
    fn roundtrip_single_object_sum_batch_first_bad_read_key() {
        roundtrip_sum_batch(true, false, true)
    }

    #[test]
    fn roundtrip_single_object_sum_batch_second_ok() {
        roundtrip_sum_batch(false, true, true)
    }

    #[test]
    fn roundtrip_single_object_sum_batch_second_bad_read_key() {
        roundtrip_sum_batch(false, false, true)
    }

    fn roundtrip_sum_batch(is_first: bool, keys_match: bool, single_object_write: bool) {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let mut batch_writer = BatchWriter::new(
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
        batch_writer.set_use_single_object_write(single_object_write);
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
        let idx = i32::from(!is_first);
        let filenames = if single_object_write {
            vec![format!("sum_{}.sig", idx)]
        } else {
            vec![
                format!("sum_{}", idx),
                format!("invalid_uuid_{}.avro", idx),
                format!("sum_{}.sig", idx),
            ]
        };
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            &*SUM_PART,
            &INVALID_PACKETS,
            batch_path,
            &filenames,
            &batch_writer,
            &batch_reader,
            &verify_transport,
            &default_ingestor_private_key(),
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
        let metrics_collector = BatchReaderMetricsCollector::new("test").unwrap();

        let mut batch_writer = BatchWriter::new(
            Batch::new_ingestion(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP),
            &write_transport,
            &DEFAULT_TRACE_ID,
        );
        batch_writer.set_use_bogus_packet_file_digest(true);

        let mut batch_reader = BatchReader::new(
            Batch::new_ingestion(AGGREGATION_NAME, &BATCH_ID, &END_TIMESTAMP),
            &read_transport,
            true, // permit_malformed_batch
            &DEFAULT_TRACE_ID,
            &logger,
        );
        batch_reader.set_metrics_collector(&metrics_collector);

        let write_key = default_ingestor_private_key();
        batch_writer
            .write(
                &write_key,
                IngestionHeader {
                    batch_uuid: BATCH_ID,
                    name: AGGREGATION_NAME.to_string(),
                    bins: 2,
                    epsilon: 1.601,
                    prime: 17,
                    number_of_servers: 2,
                    hamming_weight: None,
                    batch_start_time: 789456123,
                    batch_end_time: 789456321,
                    packet_file_digest: Vec::new(),
                },
                Some(IngestionDataSharePacket {
                    uuid: Uuid::new_v4(),
                    encrypted_payload: vec![0u8, 1u8, 2u8, 3u8],
                    encryption_key_id: Some("fake-key-1".to_owned()),
                    r_pit: 1,
                    version_configuration: Some("config-1".to_owned()),
                    device_nonce: None,
                }),
            )
            .expect("Couldn't write batch");

        // Verify with different key than we signed with
        let mut key_map = HashMap::new();
        key_map.insert(
            write_key.identifier,
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
