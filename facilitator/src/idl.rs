use crate::intake::IntakeError;
use avro_rs::{
    from_value,
    types::{Record, Value, ValueKind},
    Reader, Schema, Writer,
};
use lazy_static::lazy_static;
use prio::{
    field::FieldPriov2,
    server::{Server, ServerError, VerificationMessage},
};
use serde::{Deserialize, Serialize};
use std::{
    convert::TryFrom,
    fmt::Display,
    io::{Read, Write},
    num::TryFromIntError,
};
use uuid::Uuid;

lazy_static! {
    // Schema::parse_list returns its schemas in the order they are provided;
    // for ease-of-use, we pull these out into their own helpfully-named
    // variables. `_SCHEMAS` should be referenced only within this
    // `lazy_static` block (and ideally would not exist at all). Schema users
    // should refer to the various `&'static Schema` exported below.
    static ref _SCHEMAS: Vec<Schema> = {
        const BATCH_SIGNATURE_SCHEMA: &str = include_str!("../../avro-schema/batch-signature.avsc");
        const INGESTION_HEADER_SCHEMA: &str =
            include_str!("../../avro-schema/ingestion-header.avsc");
        const INGESTION_DATA_SHARE_PACKET_SCHEMA: &str =
            include_str!("../../avro-schema/ingestion-data-share-packet.avsc");
        const VALIDATION_HEADER_SCHEMA: &str =
            include_str!("../../avro-schema/validation-header.avsc");
        const VALIDATION_PACKET_SCHEMA: &str =
            include_str!("../../avro-schema/validation-packet.avsc");
        const SUM_PART_SCHEMA: &str = include_str!("../../avro-schema/sum-part.avsc");
        const INVALID_PACKET_SCHEMA: &str = include_str!("../../avro-schema/invalid-packet.avsc");

        Schema::parse_list(&[
            BATCH_SIGNATURE_SCHEMA,
            INGESTION_HEADER_SCHEMA,
            INGESTION_DATA_SHARE_PACKET_SCHEMA,
            VALIDATION_HEADER_SCHEMA,
            VALIDATION_PACKET_SCHEMA,
            SUM_PART_SCHEMA,
            INVALID_PACKET_SCHEMA,
        ])
        .unwrap()
    };
    static ref BATCH_SIGNATURE_SCHEMA: &'static Schema = &_SCHEMAS[0];
    static ref INGESTION_HEADER_SCHEMA: &'static Schema = &_SCHEMAS[1];
    static ref INGESTION_DATA_SHARE_PACKET_SCHEMA: &'static Schema = &_SCHEMAS[2];
    static ref VALIDATION_HEADER_SCHEMA: &'static Schema = &_SCHEMAS[3];
    static ref VALIDATION_PACKET_SCHEMA: &'static Schema = &_SCHEMAS[4];
    static ref SUM_PART_SCHEMA: &'static Schema = &_SCHEMAS[5];
    static ref INVALID_PACKET_SCHEMA: &'static Schema = &_SCHEMAS[6];
}

/// Provides additional information on what kind of Avro-related operation raised an error.
#[derive(Debug)]
pub enum AvroErrorContext {
    /// Error while constructing a `Reader`, and reading the Avro header.
    ReadHeader,
    /// Error while reading a record.
    ReadRecord,
    /// Error while writing a record out to a `Writer`.
    Append,
    /// Error while flushing a `Writer`.
    Flush,
    /// Error while deserializing from a `Value` to an application-specific data model.
    Deserialization,
}

impl Display for AvroErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AvroErrorContext::ReadHeader => write!(f, "reading avro header"),
            AvroErrorContext::ReadRecord => write!(f, "reading record"),
            AvroErrorContext::Append => write!(f, "writing"),
            AvroErrorContext::Flush => write!(f, "flushing"),
            AvroErrorContext::Deserialization => write!(f, "deserializing"),
        }
    }
}

/// Identifies which application-level validation rule resulted in a malformed header error.
#[derive(Debug)]
pub enum MalformedHeaderCause {
    /// The `batch_header_signature` field was missing.
    MissingBatchHeaderSignature,
    /// The `key_identifier` field was missing.
    MissingKeyIdentifier,
    /// The `hamming_weight` union contained a value of an invalid type.
    HammingWeightWrongType(ValueKind),
    /// There was an extra field in the header record (or a field of the wrong type).
    ExtraField(String, ValueKind),
    /// A required field was missing from the header record.
    MissingField,
    /// The `sum` array field contained a value of an invalid type.
    SumArrayElementWrongType(ValueKind),
    /// The `batch_uuids` array field contained a value of an invalid type.
    BatchUuidsElementWrongType(ValueKind),
}

impl Display for MalformedHeaderCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MalformedHeaderCause::MissingBatchHeaderSignature => {
                write!(f, "missing batch_header_signature")
            }
            MalformedHeaderCause::MissingKeyIdentifier => write!(f, "missing key_identifier"),
            MalformedHeaderCause::HammingWeightWrongType(value_kind) => {
                write!(f, "unexpected value {:?} for hamming weight", value_kind)
            }
            MalformedHeaderCause::ExtraField(name, value_kind) => {
                write!(f, "unexpected field {} -> {:?} in record", name, value_kind)
            }
            MalformedHeaderCause::MissingField => write!(f, "missing field(s) in record"),
            MalformedHeaderCause::SumArrayElementWrongType(value_kind) => {
                write!(f, "unexpected value in sum array {:?}", value_kind)
            }
            MalformedHeaderCause::BatchUuidsElementWrongType(value_kind) => {
                write!(f, "unexpected value in batch_uuids array {:?}", value_kind)
            }
        }
    }
}

/// Identifies which application-level validation rule resulted in a malformed data packet error.
#[derive(Debug)]
pub enum MalformedPacketCause {
    /// The `uuid` field was missing from the data packet record.
    MissingUuid,
    /// The `encrypted_payload` field was missing from the data packet record.
    MissingEncryptedPayload,
    /// The `r_pit` field was missing from the data packet record.
    MissingRPit,
}

impl Display for MalformedPacketCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MalformedPacketCause::MissingUuid => write!(f, "missing uuid"),
            MalformedPacketCause::MissingEncryptedPayload => write!(f, "missing encrypted_payload"),
            MalformedPacketCause::MissingRPit => write!(f, "missing r_pit"),
        }
    }
}

/// Errors related to the ENPA IDL layer, including Avro decoding, required headers, etc.
#[derive(Debug, thiserror::Error)]
pub enum IdlError {
    /// Parsing or I/O error from the Avro library.
    #[error("avro error upon {1}: {0}")]
    Avro(avro_rs::Error, AvroErrorContext),
    /// A record was expected in the Avro file, but the end of the file was found first.
    #[error("end of file")]
    Eof,
    /// A header could not be parsed, due to missing record fields or record fields of the wrong type.
    #[error("malformed header: {0}")]
    MalformedHeader(MalformedHeaderCause),
    /// A data packet could not be parsed, due to missing record fields.
    #[error("malformed data packet: {0}")]
    MalformedPacket(MalformedPacketCause),
    /// Unexpected extra values were found in an Avro file.
    #[error("excess value")]
    ExtraData,
    /// The Avro file did not contain a record as its top-level value.
    #[error("not a record")]
    WrongValueType,
    /// The value for r_pit was out of range.
    #[error("illegal r_pit value {0}")]
    OverflowingRPit(i64),
}

pub trait Header: Sized {
    /// Sets the SHA256 digest of the packet file this header describes.
    fn set_packet_file_digest<D: Into<Vec<u8>>>(&mut self, digest: D);
    /// Returns the SHA256 digest of the packet file this header describes.
    fn packet_file_digest(&self) -> &Vec<u8>;
    /// Reads and parses one Header from the provided std::io::Read instance.
    fn read<R: Read>(reader: R) -> Result<Self, IdlError>;
    /// Serializes this message into Avro format and writes it to the provided
    /// std::io::Write instance.
    fn write<W: Write>(&self, writer: &mut W) -> Result<(), IdlError>;
}

pub trait Packet: Sized + TryFrom<Value, Error = IdlError> {
    /// Serializes and writes a single Packet to the provided avro_rs::Writer.
    /// Note that unlike other structures, this does not take a primitive
    /// std::io::Write, because we do not want to create a new Avro schema and
    /// reader for each packet. The Reader must have been created with the
    /// schema returned from Packet::schema.
    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), IdlError>;

    /// Provides the avro_rs::Schema used by the packet. For constructing the
    /// avro_rs::{Reader, Writer} to use in Packet::{read, write}.
    fn schema() -> &'static Schema;
}

/// The file containing signatures over the ingestion batch header and packet
/// file.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct BatchSignature {
    pub batch_header_signature: Vec<u8>,
    pub key_identifier: String,
    pub batch_header_bytes: Option<Vec<u8>>,
    pub packet_bytes: Option<Vec<u8>>,
}

impl BatchSignature {
    /// Reads and parses one BatchSignature from the provided std::io::Read
    /// instance.
    pub fn read<R: Read>(reader: R) -> Result<BatchSignature, IdlError> {
        let mut reader = Reader::with_schema(*BATCH_SIGNATURE_SCHEMA, reader)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::ReadHeader))?;

        // We expect exactly one record and for it to be an ingestion signature
        let value = match reader.next() {
            Some(Ok(value)) => value,
            Some(Err(e)) => {
                return Err(IdlError::Avro(e, AvroErrorContext::ReadRecord));
            }
            None => return Err(IdlError::Eof),
        };
        if reader.next().is_some() {
            return Err(IdlError::ExtraData);
        }

        BatchSignature::try_from(value)
    }

    /// Serializes this signature into Avro format and writes it to the provided
    /// std::io::Write instance.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), IdlError> {
        let mut writer = Writer::new(*BATCH_SIGNATURE_SCHEMA, writer);
        writer
            .append(self.clone())
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Append))?;

        writer
            .flush()
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Flush))?;

        Ok(())
    }
}

impl TryFrom<Value> for BatchSignature {
    type Error = IdlError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let fields = if let Value::Record(f) = value {
            f
        } else {
            return Err(IdlError::WrongValueType);
        };

        // Here we might wish to use from_value::<BatchSignature>(record) but
        // avro_rs does not seem to recognize it as a Bytes and fails to
        // deserialize it. The value we unwrapped from reader.next above is a
        // vector of (String, avro_rs::Value) tuples, which we now iterate to
        // find the struct members.
        let mut batch_header_signature = None;
        let mut key_identifier = None;
        let mut batch_header_bytes = None;
        let mut packet_bytes = None;

        for (field_name, field_value) in fields {
            match (field_name.as_str(), field_value) {
                ("batch_header_signature", Value::Bytes(v)) => batch_header_signature = Some(v),
                ("key_identifier", Value::String(v)) => key_identifier = Some(v),
                ("batch_header", Value::Union(v)) => {
                    if let Value::Bytes(b) = *v {
                        batch_header_bytes = Some(b)
                    }
                }
                ("packets", Value::Union(v)) => {
                    if let Value::Bytes(b) = *v {
                        packet_bytes = Some(b)
                    }
                }
                _ => (),
            }
        }

        Ok(BatchSignature {
            batch_header_signature: batch_header_signature.ok_or(IdlError::MalformedHeader(
                MalformedHeaderCause::MissingBatchHeaderSignature,
            ))?,
            key_identifier: key_identifier.ok_or(IdlError::MalformedHeader(
                MalformedHeaderCause::MissingKeyIdentifier,
            ))?,
            batch_header_bytes,
            packet_bytes,
        })
    }
}

impl From<BatchSignature> for Value {
    fn from(sig: BatchSignature) -> Value {
        // avro_rs docs say this can only happen "if the `Schema is not
        // a `Schema::Record` variant", which shouldn't ever happen, so
        // panic for debugging
        // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
        let mut record = Record::new(*BATCH_SIGNATURE_SCHEMA)
            .expect("Unable to create record from batch signature schema");

        record.put(
            "batch_header_signature",
            Value::Bytes(sig.batch_header_signature),
        );
        record.put("key_identifier", Value::String(sig.key_identifier));
        record.put(
            "batch_header",
            Value::Union(Box::new(
                sig.batch_header_bytes.map_or(Value::Null, Value::Bytes),
            )),
        );
        record.put(
            "packets",
            Value::Union(Box::new(sig.packet_bytes.map_or(Value::Null, Value::Bytes))),
        );

        Value::Record(record.fields)
    }
}

/// The header on a Prio ingestion batch.
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct IngestionHeader {
    pub batch_uuid: Uuid,
    pub name: String,
    pub bins: i32,
    pub epsilon: f64,
    pub prime: i64,
    pub number_of_servers: i32,
    pub hamming_weight: Option<i32>,
    pub batch_start_time: i64,
    pub batch_end_time: i64,
    pub packet_file_digest: Vec<u8>,
}

impl IngestionHeader {
    #[allow(clippy::float_cmp)]
    pub fn check_parameters(&self, validation_header: &ValidationHeader) -> bool {
        self.batch_uuid == validation_header.batch_uuid
            && self.name == validation_header.name
            && self.bins == validation_header.bins
            && self.epsilon == validation_header.epsilon
            && self.prime == validation_header.prime
            && self.number_of_servers == validation_header.number_of_servers
            && self.hamming_weight == validation_header.hamming_weight
    }
}

impl Header for IngestionHeader {
    fn set_packet_file_digest<D: Into<Vec<u8>>>(&mut self, digest: D) {
        self.packet_file_digest = digest.into();
    }

    fn packet_file_digest(&self) -> &Vec<u8> {
        &self.packet_file_digest
    }

    fn read<R: Read>(reader: R) -> Result<IngestionHeader, IdlError> {
        let mut reader = Reader::with_schema(*INGESTION_HEADER_SCHEMA, reader)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::ReadHeader))?;

        // We expect exactly one record in the reader and for it to be an ingestion header
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(IdlError::WrongValueType);
            }
            Some(Err(e)) => return Err(IdlError::Avro(e, AvroErrorContext::ReadRecord)),
            None => return Err(IdlError::Eof),
        };
        if reader.next().is_some() {
            return Err(IdlError::ExtraData);
        }

        // Here we might wish to use from_value::<IngestionHeader>(record) but avro_rs does not
        // seem to recognize it as a Bytes and fails to deserialize it. The value we unwrapped from
        // reader.next above is a vector of (String, avro_rs::Value) tuples, which we now iterate to
        // find the struct members.
        let mut batch_uuid = None;
        let mut name = None;
        let mut bins = None;
        let mut epsilon = None;
        let mut prime = None;
        let mut number_of_servers = None;
        let mut hamming_weight = None;
        let mut batch_start_time = None;
        let mut batch_end_time = None;
        let mut packet_file_digest = None;

        for tuple in record {
            match (tuple.0.as_str(), tuple.1) {
                ("batch_uuid", Value::Uuid(v)) => batch_uuid = Some(v),
                ("name", Value::String(v)) => name = Some(v),
                ("bins", Value::Int(v)) => bins = Some(v),
                ("epsilon", Value::Double(v)) => epsilon = Some(v),
                ("prime", Value::Long(v)) => prime = Some(v),
                ("number_of_servers", Value::Int(v)) => number_of_servers = Some(v),
                ("hamming_weight", Value::Union(boxed)) => {
                    hamming_weight = match *boxed {
                        Value::Int(v) => Some(v),
                        Value::Null => None,
                        v => {
                            return Err(IdlError::MalformedHeader(
                                MalformedHeaderCause::HammingWeightWrongType(v.into()),
                            ));
                        }
                    }
                }
                ("batch_start_time", Value::TimestampMillis(v)) => batch_start_time = Some(v),
                ("batch_end_time", Value::TimestampMillis(v)) => batch_end_time = Some(v),
                ("packet_file_digest", Value::Bytes(v)) => packet_file_digest = Some(v),
                (f, v) => {
                    return Err(IdlError::MalformedHeader(MalformedHeaderCause::ExtraField(
                        f.to_owned(),
                        v.into(),
                    )))
                }
            }
        }

        if batch_uuid.is_none()
            || name.is_none()
            || bins.is_none()
            || epsilon.is_none()
            || prime.is_none()
            || number_of_servers.is_none()
            || batch_start_time.is_none()
            || batch_end_time.is_none()
            || packet_file_digest.is_none()
        {
            return Err(IdlError::MalformedHeader(
                MalformedHeaderCause::MissingField,
            ));
        }

        Ok(IngestionHeader {
            batch_uuid: batch_uuid.unwrap(),
            name: name.unwrap(),
            bins: bins.unwrap(),
            epsilon: epsilon.unwrap(),
            prime: prime.unwrap(),
            number_of_servers: number_of_servers.unwrap(),
            hamming_weight,
            batch_start_time: batch_start_time.unwrap(),
            batch_end_time: batch_end_time.unwrap(),
            packet_file_digest: packet_file_digest.unwrap(),
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> Result<(), IdlError> {
        let mut writer = Writer::new(*INGESTION_HEADER_SCHEMA, writer);

        // Ideally we would just do `writer.append_ser(self)` to use Serde serialization to write
        // the record but there seems to be some problem with serializing UUIDs, so we have to
        // construct the record.
        let mut record = match Record::new(writer.schema()) {
            Some(r) => r,
            None => {
                // avro_rs docs say this can only happen "if the `Schema is not a `Schema::Record`
                // variant", which shouldn't ever happen, so panic for debugging
                // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
                panic!("Unable to create Record from ingestion header schema");
            }
        };

        record.put("batch_uuid", Value::Uuid(self.batch_uuid));
        record.put("name", Value::String(self.name.clone()));
        record.put("bins", Value::Int(self.bins));
        record.put("epsilon", Value::Double(self.epsilon));
        record.put("prime", Value::Long(self.prime));
        record.put("number_of_servers", Value::Int(self.number_of_servers));
        match self.hamming_weight {
            Some(v) => record.put("hamming_weight", Value::Union(Box::new(Value::Int(v)))),
            None => record.put("hamming_weight", Value::Union(Box::new(Value::Null))),
        }
        record.put(
            "batch_start_time",
            Value::TimestampMillis(self.batch_start_time),
        );
        record.put(
            "batch_end_time",
            Value::TimestampMillis(self.batch_end_time),
        );
        record.put(
            "packet_file_digest",
            Value::Bytes(self.packet_file_digest.clone()),
        );

        writer
            .append(record)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Append))?;

        writer
            .flush()
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Flush))?;

        Ok(())
    }
}

/// A single packet from an ingestion batch file. Note that unlike the header
/// and signature, which are files containing a single record, the data share
/// file will contain many IngestionDataSharePacket records.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct IngestionDataSharePacket {
    pub uuid: Uuid,
    pub encrypted_payload: Vec<u8>,
    pub encryption_key_id: Option<String>,
    pub r_pit: i64,
    pub version_configuration: Option<String>,
    pub device_nonce: Option<Vec<u8>>,
}

impl Packet for IngestionDataSharePacket {
    fn schema() -> &'static Schema {
        *INGESTION_DATA_SHARE_PACKET_SCHEMA
    }

    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), IdlError> {
        // Ideally we would just do `writer.append_ser(self)` to use Serde
        // serialization to write the record but there seems to be some problem
        // with serializing UUIDs, so we have to construct the record.
        // avro_rs docs say this can only fail "if the `Schema is not a
        // `Schema::Record` variant", which shouldn't ever happen, so panic for
        // debugging
        // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
        let mut record = Record::new(writer.schema())
            .expect("Unable to create Record from ingestion data share packet schema");

        record.put("uuid", Value::Uuid(self.uuid));
        record.put(
            "encrypted_payload",
            Value::Bytes(self.encrypted_payload.clone()),
        );
        match &self.encryption_key_id {
            Some(v) => record.put(
                "encryption_key_id",
                Value::Union(Box::new(Value::String(v.to_owned()))),
            ),
            None => record.put("encryption_key_id", Value::Union(Box::new(Value::Null))),
        }
        record.put("r_pit", Value::Long(self.r_pit));
        match &self.version_configuration {
            Some(v) => record.put(
                "version_configuration",
                Value::Union(Box::new(Value::String(v.to_owned()))),
            ),
            None => record.put("version_configuration", Value::Union(Box::new(Value::Null))),
        }
        match &self.device_nonce {
            Some(v) => record.put(
                "device_nonce",
                Value::Union(Box::new(Value::Bytes(v.clone()))),
            ),
            None => record.put("device_nonce", Value::Union(Box::new(Value::Null))),
        }

        writer
            .append(record)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Append))?;

        Ok(())
    }
}

impl TryFrom<Value> for IngestionDataSharePacket {
    type Error = IdlError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let fields = if let Value::Record(f) = value {
            f
        } else {
            return Err(IdlError::WrongValueType);
        };

        // As in IngestionSignature::read_signature, we can't just deserialize
        // into a struct and must instead walk the vector of record fields.
        let mut uuid = None;
        let mut encrypted_payload = None;
        let mut encryption_key_id = None;
        let mut r_pit = None;
        let mut version_configuration = None;
        let mut device_nonce = None;

        for (field_name, field_value) in fields {
            match (field_name.as_str(), field_value) {
                ("uuid", Value::Uuid(v)) => uuid = Some(v),
                ("encrypted_payload", Value::Bytes(v)) => encrypted_payload = Some(v),
                ("encryption_key_id", Value::Union(boxed)) => {
                    if let Value::String(v) = *boxed {
                        encryption_key_id = Some(v);
                    }
                }
                ("r_pit", Value::Long(v)) => r_pit = Some(v),
                ("version_configuration", Value::Union(boxed)) => {
                    if let Value::String(v) = *boxed {
                        version_configuration = Some(v);
                    }
                }
                ("device_nonce", Value::Union(boxed)) => {
                    if let Value::Bytes(v) = *boxed {
                        device_nonce = Some(v);
                    }
                }
                _ => (),
            }
        }

        Ok(IngestionDataSharePacket {
            uuid: uuid.ok_or(IdlError::MalformedPacket(MalformedPacketCause::MissingUuid))?,
            encrypted_payload: encrypted_payload.ok_or(IdlError::MalformedPacket(
                MalformedPacketCause::MissingEncryptedPayload,
            ))?,
            encryption_key_id,
            r_pit: r_pit.ok_or(IdlError::MalformedPacket(MalformedPacketCause::MissingRPit))?,
            version_configuration,
            device_nonce,
        })
    }
}

impl IngestionDataSharePacket {
    pub(crate) fn generate_validation_packet(
        &self,
        servers: &mut Vec<Server<FieldPriov2>>,
    ) -> Result<ValidationPacket, IntakeError> {
        let r_pit = FieldPriov2::from(
            u32::try_from(self.r_pit)
                .map_err(|_| IntakeError::Idl(IdlError::OverflowingRPit(self.r_pit)))?,
        );
        // TODO(timg): if this fails for a non-empty subset of the
        // ingestion packets, do we abort handling of the entire
        // batch (as implemented currently) or should we record it
        // as an invalid UUID and emit a validation batch for the
        // other packets?
        for server in servers.iter_mut() {
            let validation_message =
                match server.generate_verification_message(r_pit, &self.encrypted_payload) {
                    Ok(m) => m,
                    Err(ServerError::Encrypt(_)) => {
                        continue;
                    }
                    Err(e) => {
                        return Err(IntakeError::PrioVerification(e));
                    }
                };
            return Ok(ValidationPacket {
                uuid: self.uuid,
                f_r: u32::from(validation_message.f_r) as i64,
                g_r: u32::from(validation_message.g_r) as i64,
                h_r: u32::from(validation_message.h_r) as i64,
            });
        }
        // If we arrive here, this packet could not be decrypted by any key we have.
        // All we can do is report this to the caller.
        Err(IntakeError::PacketDecryptionError(self.uuid))
    }
}

/// The header on a Prio validation (sometimes referred to as verification)
/// batch.
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct ValidationHeader {
    pub batch_uuid: Uuid,
    pub name: String,
    pub bins: i32,
    pub epsilon: f64,
    pub prime: i64,
    pub number_of_servers: i32,
    pub hamming_weight: Option<i32>,
    pub packet_file_digest: Vec<u8>,
}

impl ValidationHeader {
    #[allow(clippy::float_cmp)]
    pub fn check_parameters(&self, validation_header: &ValidationHeader) -> bool {
        self.batch_uuid == validation_header.batch_uuid
            && self.name == validation_header.name
            && self.bins == validation_header.bins
            && self.epsilon == validation_header.epsilon
            && self.prime == validation_header.prime
            && self.number_of_servers == validation_header.number_of_servers
            && self.hamming_weight == validation_header.hamming_weight
    }
}

impl Header for ValidationHeader {
    fn set_packet_file_digest<D: Into<Vec<u8>>>(&mut self, digest: D) {
        self.packet_file_digest = digest.into();
    }

    fn packet_file_digest(&self) -> &Vec<u8> {
        &self.packet_file_digest
    }

    fn read<R: Read>(reader: R) -> Result<ValidationHeader, IdlError> {
        let mut reader = Reader::with_schema(*VALIDATION_HEADER_SCHEMA, reader)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::ReadHeader))?;

        // We expect exactly one record in the reader and for it to be an ingestion header
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(IdlError::WrongValueType);
            }
            Some(Err(e)) => return Err(IdlError::Avro(e, AvroErrorContext::ReadRecord)),
            None => return Err(IdlError::Eof),
        };
        if reader.next().is_some() {
            return Err(IdlError::ExtraData);
        }

        // Here we might wish to use from_value::<IngestionSignature>(record) but avro_rs does not
        // seem to recognize it as a Bytes and fails to deserialize it. The value we unwrapped from
        // reader.next above is a vector of (String, avro_rs::Value) tuples, which we now iterate to
        // find the struct members.
        let mut batch_uuid = None;
        let mut name = None;
        let mut bins = None;
        let mut epsilon = None;
        let mut prime = None;
        let mut number_of_servers = None;
        let mut hamming_weight = None;
        let mut packet_file_digest = None;

        for tuple in record {
            match (tuple.0.as_str(), tuple.1) {
                ("batch_uuid", Value::Uuid(v)) => batch_uuid = Some(v),
                ("name", Value::String(v)) => name = Some(v),
                ("bins", Value::Int(v)) => bins = Some(v),
                ("epsilon", Value::Double(v)) => epsilon = Some(v),
                ("prime", Value::Long(v)) => prime = Some(v),
                ("number_of_servers", Value::Int(v)) => number_of_servers = Some(v),
                ("hamming_weight", Value::Union(boxed)) => {
                    hamming_weight = match *boxed {
                        Value::Int(v) => Some(v),
                        Value::Null => None,
                        v => {
                            return Err(IdlError::MalformedHeader(
                                MalformedHeaderCause::HammingWeightWrongType(v.into()),
                            ));
                        }
                    }
                }
                ("packet_file_digest", Value::Bytes(v)) => packet_file_digest = Some(v),
                (f, v) => {
                    return Err(IdlError::MalformedHeader(MalformedHeaderCause::ExtraField(
                        f.to_owned(),
                        v.into(),
                    )))
                }
            }
        }

        if batch_uuid.is_none()
            || name.is_none()
            || bins.is_none()
            || epsilon.is_none()
            || prime.is_none()
            || number_of_servers.is_none()
            || packet_file_digest.is_none()
        {
            return Err(IdlError::MalformedHeader(
                MalformedHeaderCause::MissingField,
            ));
        }

        Ok(ValidationHeader {
            batch_uuid: batch_uuid.unwrap(),
            name: name.unwrap(),
            bins: bins.unwrap(),
            epsilon: epsilon.unwrap(),
            prime: prime.unwrap(),
            number_of_servers: number_of_servers.unwrap(),
            hamming_weight,
            packet_file_digest: packet_file_digest.unwrap(),
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> Result<(), IdlError> {
        let mut writer = Writer::new(*VALIDATION_HEADER_SCHEMA, writer);

        let mut record = match Record::new(writer.schema()) {
            Some(r) => r,
            None => {
                panic!("Unable to create Record from ingestion header schema");
            }
        };

        record.put("batch_uuid", Value::Uuid(self.batch_uuid));
        record.put("name", Value::String(self.name.clone()));
        record.put("bins", Value::Int(self.bins));
        record.put("epsilon", Value::Double(self.epsilon));
        record.put("prime", Value::Long(self.prime));
        record.put("number_of_servers", Value::Int(self.number_of_servers));
        record.put(
            "hamming_weight",
            Value::Union(Box::new(
                self.hamming_weight.map_or(Value::Null, Value::Int),
            )),
        );
        record.put(
            "packet_file_digest",
            Value::Bytes(self.packet_file_digest.clone()),
        );

        writer
            .append(record)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Append))?;

        writer
            .flush()
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Flush))?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct ValidationPacket {
    pub uuid: Uuid,
    pub f_r: i64,
    pub g_r: i64,
    pub h_r: i64,
}

impl Packet for ValidationPacket {
    fn schema() -> &'static Schema {
        *VALIDATION_PACKET_SCHEMA
    }

    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), IdlError> {
        // Ideally we would just do `writer.append_ser(self)` to use Serde serialization to write
        // the record but there seems to be some problem with serializing UUIDs, so we have to
        // construct the record.
        let mut record = match Record::new(writer.schema()) {
            Some(r) => r,
            None => {
                // avro_rs docs say this can only happen "if the `Schema is not a `Schema::Record`
                // variant", which shouldn't ever happen, so panic for debugging
                // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
                panic!("Unable to create Record from ingestion data share packet schema");
            }
        };

        record.put("uuid", Value::Uuid(self.uuid));
        record.put("f_r", Value::Long(self.f_r));
        record.put("g_r", Value::Long(self.g_r));
        record.put("h_r", Value::Long(self.h_r));

        writer
            .append(record)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Append))?;

        Ok(())
    }
}

impl TryFrom<Value> for ValidationPacket {
    type Error = IdlError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        from_value(&value).map_err(|e| IdlError::Avro(e, AvroErrorContext::Deserialization))
    }
}

impl TryFrom<&ValidationPacket> for VerificationMessage<FieldPriov2> {
    type Error = TryFromIntError;

    fn try_from(p: &ValidationPacket) -> Result<Self, Self::Error> {
        Ok(VerificationMessage {
            f_r: FieldPriov2::from(u32::try_from(p.f_r)?),
            g_r: FieldPriov2::from(u32::try_from(p.g_r)?),
            h_r: FieldPriov2::from(u32::try_from(p.h_r)?),
        })
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct SumPart {
    pub batch_uuids: Vec<Uuid>,
    pub name: String,
    pub bins: i32,
    pub epsilon: f64,
    pub prime: i64,
    pub number_of_servers: i32,
    pub hamming_weight: Option<i32>,
    pub sum: Vec<i64>,
    pub aggregation_start_time: i64,
    pub aggregation_end_time: i64,
    pub packet_file_digest: Vec<u8>,
    pub total_individual_clients: i64,
}

impl SumPart {
    pub fn sum(&self) -> Result<Vec<FieldPriov2>, TryFromIntError> {
        self.sum
            .iter()
            .map(|i| Ok(FieldPriov2::from(u32::try_from(*i)?)))
            .collect()
    }
}

impl Header for SumPart {
    fn set_packet_file_digest<D: Into<Vec<u8>>>(&mut self, digest: D) {
        self.packet_file_digest = digest.into();
    }

    fn packet_file_digest(&self) -> &Vec<u8> {
        &self.packet_file_digest
    }

    fn read<R: Read>(reader: R) -> Result<SumPart, IdlError> {
        let mut reader = Reader::with_schema(*SUM_PART_SCHEMA, reader)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::ReadHeader))?;

        // We expect exactly one record in the reader and for it to be a sum
        // part.
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(IdlError::WrongValueType);
            }
            Some(Err(e)) => return Err(IdlError::Avro(e, AvroErrorContext::ReadRecord)),
            None => return Err(IdlError::Eof),
        };
        if reader.next().is_some() {
            return Err(IdlError::ExtraData);
        }

        let mut batch_uuids = None;
        let mut name = None;
        let mut bins = None;
        let mut epsilon = None;
        let mut prime = None;
        let mut number_of_servers = None;
        let mut hamming_weight = None;
        let mut sum = None;
        let mut aggregation_start_time = None;
        let mut aggregation_end_time = None;
        let mut packet_file_digest = None;
        let mut total_individual_clients = None;

        for tuple in record {
            match (tuple.0.as_str(), tuple.1) {
                ("batch_uuids", Value::Array(vector)) => {
                    batch_uuids = Some(
                        vector
                            .into_iter()
                            .map(|value| {
                                if let Value::Uuid(u) = value {
                                    Ok(u)
                                } else {
                                    Err(IdlError::MalformedHeader(
                                        MalformedHeaderCause::BatchUuidsElementWrongType(
                                            value.into(),
                                        ),
                                    ))
                                }
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                }
                ("name", Value::String(v)) => name = Some(v),
                ("bins", Value::Int(v)) => bins = Some(v),
                ("epsilon", Value::Double(v)) => epsilon = Some(v),
                ("prime", Value::Long(v)) => prime = Some(v),
                ("number_of_servers", Value::Int(v)) => number_of_servers = Some(v),
                ("hamming_weight", Value::Union(boxed)) => {
                    hamming_weight = match *boxed {
                        Value::Int(v) => Some(v),
                        Value::Null => None,
                        v => {
                            return Err(IdlError::MalformedHeader(
                                MalformedHeaderCause::HammingWeightWrongType(v.into()),
                            ));
                        }
                    }
                }
                ("sum", Value::Array(vector)) => {
                    sum = Some(
                        vector
                            .into_iter()
                            .map(|value| {
                                if let Value::Long(l) = value {
                                    Ok(l)
                                } else {
                                    Err(IdlError::MalformedHeader(
                                        MalformedHeaderCause::SumArrayElementWrongType(
                                            value.into(),
                                        ),
                                    ))
                                }
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                }
                ("aggregation_start_time", Value::TimestampMillis(v)) => {
                    aggregation_start_time = Some(v)
                }
                ("aggregation_end_time", Value::TimestampMillis(v)) => {
                    aggregation_end_time = Some(v)
                }
                ("packet_file_digest", Value::Bytes(v)) => packet_file_digest = Some(v),
                ("total_individual_clients", Value::Long(v)) => total_individual_clients = Some(v),
                (f, v) => {
                    return Err(IdlError::MalformedHeader(MalformedHeaderCause::ExtraField(
                        f.to_owned(),
                        v.into(),
                    )))
                }
            }
        }

        if batch_uuids.is_none()
            || name.is_none()
            || bins.is_none()
            || epsilon.is_none()
            || prime.is_none()
            || number_of_servers.is_none()
            || sum.is_none()
            || aggregation_start_time.is_none()
            || aggregation_end_time.is_none()
            || packet_file_digest.is_none()
        {
            return Err(IdlError::MalformedHeader(
                MalformedHeaderCause::MissingField,
            ));
        }

        Ok(SumPart {
            batch_uuids: batch_uuids.unwrap(),
            name: name.unwrap(),
            bins: bins.unwrap(),
            epsilon: epsilon.unwrap(),
            prime: prime.unwrap(),
            number_of_servers: number_of_servers.unwrap(),
            hamming_weight,
            sum: sum.unwrap(),
            aggregation_start_time: aggregation_start_time.unwrap(),
            aggregation_end_time: aggregation_end_time.unwrap(),
            packet_file_digest: packet_file_digest.unwrap(),
            total_individual_clients: total_individual_clients.unwrap(),
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> Result<(), IdlError> {
        let mut writer = Writer::new(*SUM_PART_SCHEMA, writer);

        // Ideally we would just do `writer.append_ser(self)` to use Serde serialization to write
        // the record but there seems to be some problem with serializing UUIDs, so we have to
        // construct the record.
        let mut record = match Record::new(writer.schema()) {
            Some(r) => r,
            None => {
                // avro_rs docs say this can only happen "if the `Schema is not a `Schema::Record`
                // variant", which shouldn't ever happen, so panic for debugging
                // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
                panic!("Unable to create Record from sum part schema");
            }
        };

        record.put(
            "batch_uuids",
            Value::Array(self.batch_uuids.iter().map(|u| Value::Uuid(*u)).collect()),
        );
        record.put("name", Value::String(self.name.clone()));
        record.put("bins", Value::Int(self.bins));
        record.put("epsilon", Value::Double(self.epsilon));
        record.put("prime", Value::Long(self.prime));
        record.put("number_of_servers", Value::Int(self.number_of_servers));
        match self.hamming_weight {
            Some(v) => record.put("hamming_weight", Value::Union(Box::new(Value::Int(v)))),
            None => record.put("hamming_weight", Value::Union(Box::new(Value::Null))),
        }
        record.put(
            "sum",
            Value::Array(self.sum.iter().map(|l| Value::Long(*l)).collect()),
        );
        record.put(
            "aggregation_start_time",
            Value::TimestampMillis(self.aggregation_start_time),
        );
        record.put(
            "aggregation_end_time",
            Value::TimestampMillis(self.aggregation_end_time),
        );
        record.put(
            "packet_file_digest",
            Value::Bytes(self.packet_file_digest.clone()),
        );
        record.put(
            "total_individual_clients",
            Value::Long(self.total_individual_clients),
        );

        writer
            .append(record)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Append))?;

        writer
            .flush()
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Flush))?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct InvalidPacket {
    pub uuid: Uuid,
}

impl Packet for InvalidPacket {
    fn schema() -> &'static Schema {
        *INVALID_PACKET_SCHEMA
    }

    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), IdlError> {
        // Ideally we would just do `writer.append_ser(self)` to use Serde serialization to write
        // the record but there seems to be some problem with serializing UUIDs, so we have to
        // construct the record.
        let mut record = match Record::new(writer.schema()) {
            Some(r) => r,
            None => {
                // avro_rs docs say this can only happen "if the `Schema is not a `Schema::Record`
                // variant", which shouldn't ever happen, so panic for debugging
                // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
                panic!("Unable to create Record from invalid packet schema");
            }
        };

        record.put("uuid", Value::Uuid(self.uuid));

        writer
            .append(record)
            .map_err(|e| IdlError::Avro(e, AvroErrorContext::Append))?;

        Ok(())
    }
}

impl TryFrom<Value> for InvalidPacket {
    type Error = IdlError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        from_value(&value).map_err(|e| IdlError::Avro(e, AvroErrorContext::Deserialization))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    lazy_static! {
        static ref GOLDEN_BATCH_SIGNATURES: Vec<(BatchSignature, Vec<u8>)> = {
            let batch_signature_1_bytes: Vec<u8> = hex::decode("4f626a0104166176726f2e736368656d61fa027b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f42617463685369676e6174757265222c226669656c6473223a5b7b226e616d65223a2262617463685f6865616465725f7369676e6174757265222c2274797065223a226279746573227d2c7b226e616d65223a226b65795f6964656e746966696572222c2274797065223a22737472696e67227d5d7d146176726f2e636f646563086e756c6c00d0c5ed8df0137655e2aa2054a0c8c62902220801020304166d792d636f6f6c2d6b6579d0c5ed8df0137655e2aa2054a0c8c629").unwrap();
            let batch_signature_1: BatchSignature = BatchSignature {
                batch_header_signature: vec![1, 2, 3, 4],
                key_identifier: "my-cool-key".to_owned(),
                batch_header_bytes: None,
                packet_bytes: None,
            };

            let batch_signature_2_bytes: Vec<u8> = hex::decode("4f626a0104146176726f2e636f646563086e756c6c166176726f2e736368656d61ec047b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f42617463685369676e6174757265222c226669656c6473223a5b7b226e616d65223a2262617463685f6865616465725f7369676e6174757265222c2274797065223a226279746573227d2c7b226e616d65223a226b65795f6964656e746966696572222c2274797065223a22737472696e67227d2c7b226e616d65223a2262617463685f686561646572222c2274797065223a5b226e756c6c222c226279746573225d2c2264656661756c74223a6e756c6c7d2c7b226e616d65223a227061636b657473222c2274797065223a5b226e756c6c222c226279746573225d2c2264656661756c74223a6e756c6c7d5d7d00c0a56e7df8f8ca71664148e45fdd15cf023a0805060708166d792d636f6f6c2d6b65790208090a0b0c02080d0e0f10c0a56e7df8f8ca71664148e45fdd15cf").unwrap();
            let batch_signature_2: BatchSignature = BatchSignature {
                batch_header_signature: vec![5, 6, 7, 8],
                key_identifier: "my-cool-key".to_owned(),
                batch_header_bytes: Some(vec![9, 10, 11, 12]),
                packet_bytes: Some(vec![13, 14, 15, 16]),
            };

            vec![
                (batch_signature_1, batch_signature_1_bytes),
                (batch_signature_2, batch_signature_2_bytes),
            ]
        };
    }

    #[test]
    fn read_batch_signature() {
        for (want_signature, signature_bytes) in &*GOLDEN_BATCH_SIGNATURES {
            let batch_signature = BatchSignature::read(&signature_bytes[..]).unwrap();
            assert_eq!(want_signature, &batch_signature)
        }
    }

    #[test]
    fn roundtrip_batch_signature() {
        for (signature, _) in &*GOLDEN_BATCH_SIGNATURES {
            let mut record_vec = Vec::new();
            signature.write(&mut record_vec).unwrap();
            let signature_again = BatchSignature::read(&record_vec[..]).unwrap();
            assert_eq!(signature, &signature_again);
        }
    }

    lazy_static! {
        static ref GOLDEN_INGESTION_HEADERS: Vec<(IngestionHeader, Vec<u8>)> = {
            let ingestion_header_1_bytes = hex::decode("4f626a0104146176726f2e636f646563086e756c6c166176726f2e736368656d61e8097b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f496e67657374696f6e486561646572222c226669656c6473223a5b7b226e616d65223a2262617463685f75756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a226e616d65222c2274797065223a22737472696e67227d2c7b226e616d65223a2262696e73222c2274797065223a22696e74227d2c7b226e616d65223a22657073696c6f6e222c2274797065223a22646f75626c65227d2c7b226e616d65223a227072696d65222c2274797065223a226c6f6e67222c2264656661756c74223a343239333931383732317d2c7b226e616d65223a226e756d6265725f6f665f73657276657273222c2274797065223a22696e74222c2264656661756c74223a327d2c7b226e616d65223a2268616d6d696e675f776569676874222c2274797065223a5b22696e74222c226e756c6c225d7d2c7b226e616d65223a2262617463685f73746172745f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a2262617463685f656e645f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a227061636b65745f66696c655f646967657374222c2274797065223a226279746573227d5d7d0010f2cebbc55708399ccbf2a3eb7673400290014862653464663164362d656638362d346339312d383838302d6137393430303238613739341466616b652d62617463680404560e2db29df93f220402f693f1f0058297f1f005020110f2cebbc55708399ccbf2a3eb767340").unwrap();
            let ingestion_header_1 = IngestionHeader {
                batch_uuid: Uuid::parse_str("be4df1d6-ef86-4c91-8880-a7940028a794").unwrap(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: None,
                batch_start_time: 789456123,
                batch_end_time: 789456321,
                packet_file_digest: vec![1u8],
            };

            let ingestion_header_2_bytes = hex::decode("4f626a0104146176726f2e636f646563086e756c6c166176726f2e736368656d61e8097b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f496e67657374696f6e486561646572222c226669656c6473223a5b7b226e616d65223a2262617463685f75756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a226e616d65222c2274797065223a22737472696e67227d2c7b226e616d65223a2262696e73222c2274797065223a22696e74227d2c7b226e616d65223a22657073696c6f6e222c2274797065223a22646f75626c65227d2c7b226e616d65223a227072696d65222c2274797065223a226c6f6e67222c2264656661756c74223a343239333931383732317d2c7b226e616d65223a226e756d6265725f6f665f73657276657273222c2274797065223a22696e74222c2264656661756c74223a327d2c7b226e616d65223a2268616d6d696e675f776569676874222c2274797065223a5b22696e74222c226e756c6c225d7d2c7b226e616d65223a2262617463685f73746172745f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a2262617463685f656e645f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a227061636b65745f66696c655f646967657374222c2274797065223a226279746573227d5d7d00e30977c811a59cb3e1c2043f8f93c1a30292014830336239646561312d623534382d343032622d393036312d3031333866376337323864621466616b652d62617463680404560e2db29df93f22040018f693f1f0058297f1f0050202e30977c811a59cb3e1c2043f8f93c1a3").unwrap();
            let ingestion_header_2 = IngestionHeader {
                batch_uuid: Uuid::parse_str("03b9dea1-b548-402b-9061-0138f7c728db").unwrap(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: Some(12),
                batch_start_time: 789456123,
                batch_end_time: 789456321,
                packet_file_digest: vec![2u8],
            };

            vec![
                (ingestion_header_1, ingestion_header_1_bytes),
                (ingestion_header_2, ingestion_header_2_bytes),
            ]
        };
    }

    #[test]
    fn read_ingestion_header() {
        for (want_header, header_bytes) in &*GOLDEN_INGESTION_HEADERS {
            let ingestion_header = IngestionHeader::read(&header_bytes[..]).unwrap();
            assert_eq!(want_header, &ingestion_header);
        }
    }

    #[test]
    fn roundtrip_ingestion_header() {
        for (header, _) in &*GOLDEN_INGESTION_HEADERS {
            let mut record_vec = Vec::new();
            header.write(&mut record_vec).expect("write error");
            let header_again = IngestionHeader::read(&record_vec[..]).expect("read error");
            assert_eq!(&header_again, header);
        }
    }

    lazy_static! {
        static ref GOLDEN_INGESTION_DATA_SHARE_PACKETS: Vec<(IngestionDataSharePacket, Vec<u8>)> = {
            let ingestion_data_share_packet_1_bytes = hex::decode("4f626a0104146176726f2e636f646563086e756c6c166176726f2e736368656d6198067b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f4461746153686172655061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a22656e637279707465645f7061796c6f6164222c2274797065223a226279746573227d2c7b226e616d65223a22656e6372797074696f6e5f6b65795f6964222c2274797065223a5b226e756c6c222c22737472696e67225d7d2c7b226e616d65223a22725f706974222c2274797065223a226c6f6e67227d2c7b226e616d65223a2276657273696f6e5f636f6e66696775726174696f6e222c2274797065223a5b226e756c6c222c22737472696e67225d7d2c7b226e616d65223a226465766963655f6e6f6e6365222c2274797065223a5b226e756c6c222c226279746573225d7d5d7d009c5da591513530367b96e748024edd3d0284014839363466373234302d643066662d343234642d383966362d3636366463306534626563630800010203021466616b652d6b65792d31020210636f6e6669672d31009c5da591513530367b96e748024edd3d").unwrap();
            let ingestion_data_share_packet_1 = IngestionDataSharePacket {
                uuid: Uuid::parse_str("964f7240-d0ff-424d-89f6-666dc0e4becc").unwrap(),
                encrypted_payload: vec![0u8, 1u8, 2u8, 3u8],
                encryption_key_id: Some("fake-key-1".to_owned()),
                r_pit: 1,
                version_configuration: Some("config-1".to_owned()),
                device_nonce: None,
            };

            let ingestion_data_share_packet_2_bytes = hex::decode("4f626a0104146176726f2e636f646563086e756c6c166176726f2e736368656d6198067b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f4461746153686172655061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a22656e637279707465645f7061796c6f6164222c2274797065223a226279746573227d2c7b226e616d65223a22656e6372797074696f6e5f6b65795f6964222c2274797065223a5b226e756c6c222c22737472696e67225d7d2c7b226e616d65223a22725f706974222c2274797065223a226c6f6e67227d2c7b226e616d65223a2276657273696f6e5f636f6e66696775726174696f6e222c2274797065223a5b226e756c6c222c22737472696e67225d7d2c7b226e616d65223a226465766963655f6e6f6e6365222c2274797065223a5b226e756c6c222c226279746573225d7d5d7d005f3567685778ee120d743d690daaaa6502664830323034623036342d353865382d346666342d613635392d3164326466636334333232340804050607000400020808090a0b5f3567685778ee120d743d690daaaa65").unwrap();
            let ingestion_data_share_packet_2 = IngestionDataSharePacket {
                uuid: Uuid::parse_str("0204b064-58e8-4ff4-a659-1d2dfcc43224").unwrap(),
                encrypted_payload: vec![4u8, 5u8, 6u8, 7u8],
                encryption_key_id: None,
                r_pit: 2,
                version_configuration: None,
                device_nonce: Some(vec![8u8, 9u8, 10u8, 11u8]),
            };

            let ingestion_data_share_packet_3_bytes = hex::decode("4f626a0104166176726f2e736368656d6198067b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f4461746153686172655061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a22656e637279707465645f7061796c6f6164222c2274797065223a226279746573227d2c7b226e616d65223a22656e6372797074696f6e5f6b65795f6964222c2274797065223a5b226e756c6c222c22737472696e67225d7d2c7b226e616d65223a22725f706974222c2274797065223a226c6f6e67227d2c7b226e616d65223a2276657273696f6e5f636f6e66696775726174696f6e222c2274797065223a5b226e756c6c222c22737472696e67225d7d2c7b226e616d65223a226465766963655f6e6f6e6365222c2274797065223a5b226e756c6c222c226279746573225d7d5d7d146176726f2e636f646563086e756c6c00ad4c03ddfc76ee72ebd1d1b9592a5d3a02724864383165646431322d376335302d343939362d613733622d3539303064333734303836620808090a0b021466616b652d6b65792d33060000ad4c03ddfc76ee72ebd1d1b9592a5d3a").unwrap();
            let ingestion_data_share_packet_3 = IngestionDataSharePacket {
                uuid: Uuid::parse_str("d81edd12-7c50-4996-a73b-5900d374086b").unwrap(),
                encrypted_payload: vec![8u8, 9u8, 10u8, 11u8],
                encryption_key_id: Some("fake-key-3".to_owned()),
                r_pit: 3,
                version_configuration: None,
                device_nonce: None,
            };

            vec![
                (
                    ingestion_data_share_packet_1,
                    ingestion_data_share_packet_1_bytes,
                ),
                (
                    ingestion_data_share_packet_2,
                    ingestion_data_share_packet_2_bytes,
                ),
                (
                    ingestion_data_share_packet_3,
                    ingestion_data_share_packet_3_bytes,
                ),
            ]
        };
    }

    #[test]
    fn read_data_share_packet() {
        for (want_data_share_packet, data_share_packet_bytes) in
            &*GOLDEN_INGESTION_DATA_SHARE_PACKETS
        {
            let mut reader = Reader::with_schema(
                IngestionDataSharePacket::schema(),
                &data_share_packet_bytes[..],
            )
            .unwrap();
            let data_share_packet =
                IngestionDataSharePacket::try_from(reader.next().unwrap().unwrap()).unwrap();
            assert_eq!(want_data_share_packet, &data_share_packet);
        }
    }

    #[test]
    fn roundtrip_data_share_packet() {
        let mut record_vec = Vec::new();

        let mut writer = Writer::new(IngestionDataSharePacket::schema(), &mut record_vec);

        for (packet, _) in &*GOLDEN_INGESTION_DATA_SHARE_PACKETS {
            packet.write(&mut writer).expect("write error");
        }
        writer.flush().unwrap();

        let mut reader =
            Reader::with_schema(IngestionDataSharePacket::schema(), &record_vec[..]).unwrap();
        for (packet, _) in &*GOLDEN_INGESTION_DATA_SHARE_PACKETS {
            let packet_again = IngestionDataSharePacket::try_from(reader.next().unwrap().unwrap())
                .expect("read error");
            assert_eq!(packet_again, *packet);
        }

        // Do one more read. This should yield EOF.
        assert_matches!(reader.next(), None);
    }

    lazy_static! {
        static ref GOLDEN_VALIDATION_HEADERS: Vec<(ValidationHeader, Vec<u8>)> = {
            let validation_header_1_bytes = hex::decode("4f626a0104166176726f2e736368656d619a077b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f56616c6964697479486561646572222c226669656c6473223a5b7b226e616d65223a2262617463685f75756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a226e616d65222c2274797065223a22737472696e67227d2c7b226e616d65223a2262696e73222c2274797065223a22696e74227d2c7b226e616d65223a22657073696c6f6e222c2274797065223a22646f75626c65227d2c7b226e616d65223a227072696d65222c2274797065223a226c6f6e67222c2264656661756c74223a343239333931383732317d2c7b226e616d65223a226e756d6265725f6f665f73657276657273222c2274797065223a22696e74222c2264656661756c74223a327d2c7b226e616d65223a2268616d6d696e675f776569676874222c2274797065223a5b22696e74222c226e756c6c225d7d2c7b226e616d65223a227061636b65745f66696c655f646967657374222c2274797065223a226279746573227d5d7d146176726f2e636f646563086e756c6c009f8fc651b8e2191bdfa8ad453ea2b73c027c4838663566306637392d303838372d343332322d386238632d3634646165623765353138331466616b652d62617463680404560e2db29df93f22040202049f8fc651b8e2191bdfa8ad453ea2b73c").unwrap();
            let validation_header_1 = ValidationHeader {
                batch_uuid: Uuid::parse_str("8f5f0f79-0887-4322-8b8c-64daeb7e5183").unwrap(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: None,
                packet_file_digest: vec![4u8],
            };

            let validation_header_2_bytes = hex::decode("4f626a0104166176726f2e736368656d619a077b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f56616c6964697479486561646572222c226669656c6473223a5b7b226e616d65223a2262617463685f75756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a226e616d65222c2274797065223a22737472696e67227d2c7b226e616d65223a2262696e73222c2274797065223a22696e74227d2c7b226e616d65223a22657073696c6f6e222c2274797065223a22646f75626c65227d2c7b226e616d65223a227072696d65222c2274797065223a226c6f6e67222c2264656661756c74223a343239333931383732317d2c7b226e616d65223a226e756d6265725f6f665f73657276657273222c2274797065223a22696e74222c2264656661756c74223a327d2c7b226e616d65223a2268616d6d696e675f776569676874222c2274797065223a5b22696e74222c226e756c6c225d7d2c7b226e616d65223a227061636b65745f66696c655f646967657374222c2274797065223a226279746573227d5d7d146176726f2e636f646563086e756c6c00d45f391c5edf41c4b15025c8a6645177027e4834636464623330322d343462322d343565392d613164312d6438383464663862383765301466616b652d62617463680404560e2db29df93f220400180206d45f391c5edf41c4b15025c8a6645177").unwrap();
            let validation_header_2 = ValidationHeader {
                batch_uuid: Uuid::parse_str("4cddb302-44b2-45e9-a1d1-d884df8b87e0").unwrap(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: Some(12),
                packet_file_digest: vec![6u8],
            };

            vec![
                (validation_header_1, validation_header_1_bytes),
                (validation_header_2, validation_header_2_bytes),
            ]
        };
    }

    #[test]
    fn read_validation_header() {
        for (want_header, header_bytes) in &*GOLDEN_VALIDATION_HEADERS {
            let validation_header = ValidationHeader::read(&header_bytes[..]).unwrap();
            assert_eq!(want_header, &validation_header);
        }
    }

    #[test]
    fn roundtrip_validation_header() {
        for (header, _) in &*GOLDEN_VALIDATION_HEADERS {
            let mut record_vec = Vec::new();
            header.write(&mut record_vec).expect("write error");
            let header_again = ValidationHeader::read(&record_vec[..]).expect("read error");
            assert_eq!(header_again, *header);
        }
    }

    lazy_static! {
        static ref GOLDEN_VALIDATION_PACKETS: Vec<(ValidationPacket, Vec<u8>)> = {
            let validation_packet_1_bytes = hex::decode("4f626a0104166176726f2e736368656d61ee037b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f56616c69646974795061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a22665f72222c2274797065223a226c6f6e67227d2c7b226e616d65223a22675f72222c2274797065223a226c6f6e67227d2c7b226e616d65223a22685f72222c2274797065223a226c6f6e67227d5d7d146176726f2e636f646563086e756c6c00368cb0feb3f809181cea03dcb7178a2502504835316239643938312d393537342d343466622d616533352d383463636631336366306639020406368cb0feb3f809181cea03dcb7178a25").unwrap();
            let validation_packet_1 = ValidationPacket {
                uuid: Uuid::parse_str("51b9d981-9574-44fb-ae35-84ccf13cf0f9").unwrap(),
                f_r: 1,
                g_r: 2,
                h_r: 3,
            };

            let validation_packet_2_bytes = hex::decode("4f626a0104146176726f2e636f646563086e756c6c166176726f2e736368656d61ee037b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f56616c69646974795061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a22665f72222c2274797065223a226c6f6e67227d2c7b226e616d65223a22675f72222c2274797065223a226c6f6e67227d2c7b226e616d65223a22685f72222c2274797065223a226c6f6e67227d5d7d00ce6c0aad9e40580f01836ff18ffc13d302504863306532343832642d656533622d343864342d623333302d356566623464643035323638080a0cce6c0aad9e40580f01836ff18ffc13d3").unwrap();
            let validation_packet_2 = ValidationPacket {
                uuid: Uuid::parse_str("c0e2482d-ee3b-48d4-b330-5efb4dd05268").unwrap(),
                f_r: 4,
                g_r: 5,
                h_r: 6,
            };

            let validation_packet_3_bytes = hex::decode("4f626a0104166176726f2e736368656d61ee037b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f56616c69646974795061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d2c7b226e616d65223a22665f72222c2274797065223a226c6f6e67227d2c7b226e616d65223a22675f72222c2274797065223a226c6f6e67227d2c7b226e616d65223a22685f72222c2274797065223a226c6f6e67227d5d7d146176726f2e636f646563086e756c6c008f37d54782b5a4ba978ca4b04c41d54102504865656365653134622d613030642d343766622d613031382d3130643562316536303739370e10128f37d54782b5a4ba978ca4b04c41d541").unwrap();
            let validation_packet_3 = ValidationPacket {
                uuid: Uuid::parse_str("eecee14b-a00d-47fb-a018-10d5b1e60797").unwrap(),
                f_r: 7,
                g_r: 8,
                h_r: 9,
            };

            vec![
                (validation_packet_1, validation_packet_1_bytes),
                (validation_packet_2, validation_packet_2_bytes),
                (validation_packet_3, validation_packet_3_bytes),
            ]
        };
    }

    #[test]
    fn read_validation_packet() {
        for (want_validation_packet, validation_packet_bytes) in &*GOLDEN_VALIDATION_PACKETS {
            let mut reader =
                Reader::with_schema(ValidationPacket::schema(), &validation_packet_bytes[..])
                    .unwrap();
            let data_share_packet =
                ValidationPacket::try_from(reader.next().unwrap().unwrap()).unwrap();
            assert_eq!(want_validation_packet, &data_share_packet);
        }
    }

    #[test]
    fn roundtrip_validation_packet() {
        let mut record_vec = Vec::new();

        let mut writer = Writer::new(ValidationPacket::schema(), &mut record_vec);

        for (packet, _) in &*GOLDEN_VALIDATION_PACKETS {
            packet.write(&mut writer).expect("write error");
        }
        writer.flush().unwrap();

        let mut reader = Reader::with_schema(ValidationPacket::schema(), &record_vec[..]).unwrap();
        for (packet, _) in &*GOLDEN_VALIDATION_PACKETS {
            let packet_again =
                ValidationPacket::try_from(reader.next().unwrap().unwrap()).expect("read error");
            assert_eq!(packet_again, *packet);
        }

        // Do one more read. This should yield EOF.
        assert_matches!(reader.next(), None);
    }

    lazy_static! {
        static ref GOLDEN_SUM_PARTS: Vec<(SumPart, Vec<u8>)> = {
            let sum_part_1_bytes = hex::decode("4f626a0104166176726f2e736368656d61b20b7b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f53756d50617274222c226669656c6473223a5b7b226e616d65223a2262617463685f7575696473222c2274797065223a7b2274797065223a226172726179222c226974656d73223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d7d2c7b226e616d65223a226e616d65222c2274797065223a22737472696e67227d2c7b226e616d65223a2262696e73222c2274797065223a22696e74227d2c7b226e616d65223a22657073696c6f6e222c2274797065223a22646f75626c65227d2c7b226e616d65223a227072696d65222c2274797065223a226c6f6e67227d2c7b226e616d65223a226e756d6265725f6f665f73657276657273222c2274797065223a22696e74227d2c7b226e616d65223a2268616d6d696e675f776569676874222c2274797065223a5b22696e74222c226e756c6c225d7d2c7b226e616d65223a2273756d222c2274797065223a7b2274797065223a226172726179222c226974656d73223a226c6f6e67227d7d2c7b226e616d65223a226167677265676174696f6e5f73746172745f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a226167677265676174696f6e5f656e645f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a227061636b65745f66696c655f646967657374222c2274797065223a226279746573227d2c7b226e616d65223a22746f74616c5f696e646976696475616c5f636c69656e7473222c2274797065223a226c6f6e67227d5d7d146176726f2e636f646563086e756c6c00ae60e74a8b6b11559ed0c9e7de221c0102ee01044864623562316135632d623563362d346439332d623736392d3766353765366361663538644830363265373931622d396132312d343466332d383561352d656335323333643636383662001466616b652d62617463680404560e2db29df93f22040206181a1c00f693f1f0058297f1f0050601020304ae60e74a8b6b11559ed0c9e7de221c01").unwrap();
            let sum_part_1 = SumPart {
                batch_uuids: vec![
                    Uuid::parse_str("db5b1a5c-b5c6-4d93-b769-7f57e6caf58d").unwrap(),
                    Uuid::parse_str("062e791b-9a21-44f3-85a5-ec5233d6686b").unwrap(),
                ],
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: None,
                sum: vec![12, 13, 14],
                aggregation_start_time: 789456123,
                aggregation_end_time: 789456321,
                packet_file_digest: vec![1, 2, 3],
                total_individual_clients: 2,
            };

            let sum_part_2_bytes = hex::decode("4f626a0104166176726f2e736368656d61b20b7b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f53756d50617274222c226669656c6473223a5b7b226e616d65223a2262617463685f7575696473222c2274797065223a7b2274797065223a226172726179222c226974656d73223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d7d2c7b226e616d65223a226e616d65222c2274797065223a22737472696e67227d2c7b226e616d65223a2262696e73222c2274797065223a22696e74227d2c7b226e616d65223a22657073696c6f6e222c2274797065223a22646f75626c65227d2c7b226e616d65223a227072696d65222c2274797065223a226c6f6e67227d2c7b226e616d65223a226e756d6265725f6f665f73657276657273222c2274797065223a22696e74227d2c7b226e616d65223a2268616d6d696e675f776569676874222c2274797065223a5b22696e74222c226e756c6c225d7d2c7b226e616d65223a2273756d222c2274797065223a7b2274797065223a226172726179222c226974656d73223a226c6f6e67227d7d2c7b226e616d65223a226167677265676174696f6e5f73746172745f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a226167677265676174696f6e5f656e645f74696d65222c2274797065223a7b2274797065223a226c6f6e67222c226c6f676963616c54797065223a2274696d657374616d702d6d696c6c6973227d7d2c7b226e616d65223a227061636b65745f66696c655f646967657374222c2274797065223a226279746573227d2c7b226e616d65223a22746f74616c5f696e646976696475616c5f636c69656e7473222c2274797065223a226c6f6e67227d5d7d146176726f2e636f646563086e756c6c0043a570b58e473dfef2dfd26b0bff5e7902a601024835643662343237612d393932372d343137662d383035622d373765373666323663343738001466616b652d62617463680404560e2db29df93f2204001806181a1c00f693f1f0058297f1f005060708090443a570b58e473dfef2dfd26b0bff5e79").unwrap();
            let sum_part_2 = SumPart {
                batch_uuids: vec![Uuid::parse_str("5d6b427a-9927-417f-805b-77e76f26c478").unwrap()],
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: Some(12),
                sum: vec![12, 13, 14],
                aggregation_start_time: 789456123,
                aggregation_end_time: 789456321,
                packet_file_digest: vec![7, 8, 9],
                total_individual_clients: 2,
            };

            vec![
                (sum_part_1, sum_part_1_bytes),
                (sum_part_2, sum_part_2_bytes),
            ]
        };
    }

    #[test]
    fn read_sum_part() {
        for (want_sum_part, sum_part_bytes) in &*GOLDEN_SUM_PARTS {
            let sum_part = SumPart::read(&sum_part_bytes[..]).unwrap();
            assert_eq!(want_sum_part, &sum_part);
        }
    }

    #[test]
    fn roundtrip_sum_part() {
        for (sum_part, _) in &*GOLDEN_SUM_PARTS {
            let mut record_vec = Vec::new();
            sum_part.write(&mut record_vec).expect("write error");
            let sum_part_again = SumPart::read(&record_vec[..]).expect("read error");
            assert_eq!(sum_part_again, *sum_part);
        }
    }

    lazy_static! {
        static ref GOLDEN_INVALID_PACKETS: Vec<(InvalidPacket, Vec<u8>)> = {
            let invalid_packet_1_bytes = hex::decode("4f626a0104146176726f2e636f646563086e756c6c166176726f2e736368656d61be027b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f496e76616c69645061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d5d7d006de170c446ec2172338211e7aa0ad82f024a4861623763326435622d353633342d343730662d383162352d3063656164303639616162346de170c446ec2172338211e7aa0ad82f").unwrap();
            let invalid_packet_1 = InvalidPacket {
                uuid: Uuid::parse_str("ab7c2d5b-5634-470f-81b5-0cead069aab4").unwrap(),
            };

            let invalid_packet_2_bytes = hex::decode("4f626a0104166176726f2e736368656d61be027b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f496e76616c69645061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d5d7d146176726f2e636f646563086e756c6c0095969d036d8e3d0f39a1393b221e6aae024a4839356265386139302d313232642d343663632d396361372d66366535663934316161376295969d036d8e3d0f39a1393b221e6aae").unwrap();
            let invalid_packet_2 = InvalidPacket {
                uuid: Uuid::parse_str("95be8a90-122d-46cc-9ca7-f6e5f941aa7b").unwrap(),
            };

            let invalid_packet_3_bytes = hex::decode("4f626a0104166176726f2e736368656d61be027b2274797065223a227265636f7264222c226e616d657370616365223a226f72672e61626574746572696e7465726e65742e7072696f2e7631222c226e616d65223a225072696f496e76616c69645061636b6574222c226669656c6473223a5b7b226e616d65223a2275756964222c2274797065223a7b2274797065223a22737472696e67222c226c6f676963616c54797065223a2275756964227d7d5d7d146176726f2e636f646563086e756c6c00d8f3eec9d4c68061ec330d794ea704ad024a4837366162353839362d636132352d343432632d396338302d633463663766396164393835d8f3eec9d4c68061ec330d794ea704ad").unwrap();
            let invalid_packet_3 = InvalidPacket {
                uuid: Uuid::parse_str("76ab5896-ca25-442c-9c80-c4cf7f9ad985").unwrap(),
            };

            vec![
                (invalid_packet_1, invalid_packet_1_bytes),
                (invalid_packet_2, invalid_packet_2_bytes),
                (invalid_packet_3, invalid_packet_3_bytes),
            ]
        };
    }

    #[test]
    fn read_invalid_packet() {
        for (want_invalid_packet, invalid_packet_bytes) in &*GOLDEN_INVALID_PACKETS {
            let mut reader =
                Reader::with_schema(InvalidPacket::schema(), &invalid_packet_bytes[..]).unwrap();
            let invalid_packet = InvalidPacket::try_from(reader.next().unwrap().unwrap()).unwrap();
            assert_eq!(want_invalid_packet, &invalid_packet);
        }
    }

    #[test]
    fn roundtrip_invalid_packet() {
        let mut record_vec = Vec::new();

        let mut writer = Writer::new(InvalidPacket::schema(), &mut record_vec);

        for (packet, _) in &*GOLDEN_INVALID_PACKETS {
            packet.write(&mut writer).expect("write error");
        }
        writer.flush().unwrap();

        let mut reader = Reader::with_schema(InvalidPacket::schema(), &record_vec[..]).unwrap();
        for (packet, _) in &*GOLDEN_INVALID_PACKETS {
            let packet_again =
                InvalidPacket::try_from(reader.next().unwrap().unwrap()).expect("read error");
            assert_eq!(packet_again, *packet);
        }

        // Do one more read. This should yield EOF.
        assert_matches!(reader.next(), None);
    }
}
