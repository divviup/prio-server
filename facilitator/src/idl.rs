use crate::Error;
use avro_rs::{
    from_value,
    types::{Record, Value},
    Reader, Schema, Writer,
};
use prio::{finite_field::Field, server::VerificationMessage};
use serde::{Deserialize, Serialize};
use std::{
    convert::TryFrom,
    io::{Read, Write},
    num::TryFromIntError,
};
use uuid::Uuid;

const BATCH_SIGNATURE_SCHEMA: &str = include_str!("../../avro-schema/batch-signature.avsc");
const INGESTION_HEADER_SCHEMA: &str = include_str!("../../avro-schema/ingestion-header.avsc");
const INGESTION_DATA_SHARE_PACKET_SCHEMA: &str =
    include_str!("../../avro-schema/ingestion-data-share-packet.avsc");
const VALIDATION_HEADER_SCHEMA: &str = include_str!("../../avro-schema/validation-header.avsc");
const VALIDATION_PACKET_SCHEMA: &str = include_str!("../../avro-schema/validation-packet.avsc");
const SUM_PART_SCHEMA: &str = include_str!("../../avro-schema/sum-part.avsc");
const INVALID_PACKET_SCHEMA: &str = include_str!("../../avro-schema/invalid-packet.avsc");

pub trait Header: Sized {
    /// Returns the SHA256 digest of the packet file this header describes.
    fn packet_file_digest(&self) -> &Vec<u8>;
    /// Reads and parses one Header from the provided std::io::Read instance.
    fn read<R: Read>(reader: R) -> Result<Self, Error>;
    /// Serializes this message into Avro format and writes it to the provided
    /// std::io::Write instance.
    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error>;
}

pub trait Packet: Sized {
    /// Reads and parses a single Packet from the provided avro_rs::Reader. Note
    /// that unlike other structures, this does not take a primitive
    /// std::io::Read, because we do not want to create a new Avro schema and
    /// reader for each packet. The Reader must have been created with the
    //// schema returned from Packet::schema.
    fn read<R: Read>(reader: &mut Reader<R>) -> Result<Self, Error>;

    /// Serializes and writes a single Packet to the provided avro_rs::Writer.
    /// Note that unlike other structures, this does not take a primitive
    /// std::io::Write, because we do not want to create a new Avro schema and
    /// reader for each packet. The Reader must have been created with the
    /// schema returned from Packet::schema.
    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), Error>;

    /// Implementations of Packet should return their Avro schemas as strings
    /// from this method.
    fn schema_raw() -> &'static str;

    /// Creates an avro_rs::Schema from the packet schema. For constructing the
    /// avro_rs::{Reader, Writer} to use in Packet::{read, write}. Since this
    /// only ever uses a schema whose correctness we can guarantee, it panics on
    /// failure.
    fn schema() -> Schema {
        Schema::parse_str(Self::schema_raw()).unwrap()
    }
}

/// The file containing signatures over the ingestion batch header and packet
/// file.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct BatchSignature {
    pub batch_header_signature: Vec<u8>,
    pub key_identifier: String,
}

impl BatchSignature {
    /// Reads and parses one BatchSignature from the provided std::io::Read
    /// instance.
    pub fn read<R: Read>(reader: R) -> Result<BatchSignature, Error> {
        let schema = Schema::parse_str(BATCH_SIGNATURE_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse ingestion signature schema".to_owned(), e))?;
        let mut reader = Reader::with_schema(&schema, reader)
            .map_err(|e| Error::Avro("failed to create Avro reader".to_owned(), e))?;

        // We expect exactly one record and for it to be an ingestion signature
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => return Err(Error::MalformedHeader("value is not a record".to_owned())),
            Some(Err(e)) => {
                return Err(Error::Avro(
                    "failed to read record from Avro reader".to_owned(),
                    e,
                ));
            }
            None => return Err(Error::Eof),
        };
        if reader.next().is_some() {
            return Err(Error::MalformedHeader("excess value in reader".to_owned()));
        }

        // Here we might wish to use from_value::<BatchSignature>(record) but
        // avro_rs does not seem to recognize it as a Bytes and fails to
        // deserialize it. The value we unwrapped from reader.next above is a
        // vector of (String, avro_rs::Value) tuples, which we now iterate to
        // find the struct members.
        let mut batch_header_signature = None;
        let mut key_identifier = None;

        for tuple in record {
            match (tuple.0.as_str(), tuple.1) {
                ("batch_header_signature", Value::Bytes(v)) => batch_header_signature = Some(v),
                ("key_identifier", Value::String(v)) => key_identifier = Some(v),
                (f, _) => {
                    return Err(Error::MalformedHeader(format!(
                        "unexpected field {} in record",
                        f
                    )))
                }
            }
        }

        if batch_header_signature.is_none() || key_identifier.is_none() {
            return Err(Error::MalformedHeader(
                "missing fields in record".to_owned(),
            ));
        }

        Ok(BatchSignature {
            batch_header_signature: batch_header_signature.unwrap(),
            key_identifier: key_identifier.unwrap(),
        })
    }

    /// Serializes this signature into Avro format and writes it to the provided
    /// std::io::Write instance.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(BATCH_SIGNATURE_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse ingestion signature schema".to_owned(), e))?;
        let mut writer = Writer::new(&schema, writer);

        let mut record = match Record::new(writer.schema()) {
            Some(r) => r,
            None => {
                // avro_rs docs say this can only happen "if the `Schema is not
                // a `Schema::Record` variant", which shouldn't ever happen, so
                // panic for debugging
                // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
                panic!("Unable to create Record from ingestion signature schema");
            }
        };

        record.put(
            "batch_header_signature",
            Value::Bytes(self.batch_header_signature.clone()),
        );
        record.put("key_identifier", Value::String(self.key_identifier.clone()));

        writer
            .append(record)
            .map_err(|e| Error::Avro("failed to append record to Avro writer".to_owned(), e))?;

        writer
            .flush()
            .map_err(|e| Error::Avro("failed to flush Avro writer".to_owned(), e))?;

        Ok(())
    }
}

/// The header on a Prio ingestion batch.
#[derive(Debug, Deserialize, Serialize, PartialEq)]
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
    fn packet_file_digest(&self) -> &Vec<u8> {
        &self.packet_file_digest
    }

    fn read<R: Read>(reader: R) -> Result<IngestionHeader, Error> {
        let schema = Schema::parse_str(INGESTION_HEADER_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse ingestion header schema".to_owned(), e))?;
        let mut reader = Reader::with_schema(&schema, reader).map_err(|e| {
            Error::Avro("failed to create reader for ingestion header".to_owned(), e)
        })?;

        // We expect exactly one record in the reader and for it to be an ingestion header
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => return Err(Error::MalformedHeader("value is not a record".to_owned())),
            Some(Err(e)) => {
                return Err(Error::Avro(
                    "failed to read header from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::Eof),
        };
        if reader.next().is_some() {
            return Err(Error::MalformedHeader("excess header in reader".to_owned()));
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
                            return Err(Error::MalformedHeader(format!(
                                "unexpected value {:?} for hamming weight",
                                v
                            )));
                        }
                    }
                }
                ("batch_start_time", Value::TimestampMillis(v)) => batch_start_time = Some(v),
                ("batch_end_time", Value::TimestampMillis(v)) => batch_end_time = Some(v),
                ("packet_file_digest", Value::Bytes(v)) => packet_file_digest = Some(v),
                (f, v) => {
                    return Err(Error::MalformedHeader(format!(
                        "unexpected field {} -> {:?} in record",
                        f, v
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
            return Err(Error::MalformedHeader(
                "missing field(s) in record".to_owned(),
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

    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(INGESTION_HEADER_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse ingestion header schema".to_owned(), e))?;
        let mut writer = Writer::new(&schema, writer);

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
            .map_err(|e| Error::Avro("failed to append record to Avro writer".to_owned(), e))?;

        writer
            .flush()
            .map_err(|e| Error::Avro("failed to flush Avro writer".to_owned(), e))?;

        Ok(())
    }
}

/// A single packet from an ingestion batch file. Note that unlike the header
/// and signature, which are files containing a single record, the data share
/// file will contain many IngestionDataSharePacket records.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct IngestionDataSharePacket {
    pub uuid: Uuid,
    pub encrypted_payload: Vec<u8>,
    pub encryption_key_id: Option<String>,
    pub r_pit: i64,
    pub version_configuration: Option<String>,
    pub device_nonce: Option<Vec<u8>>,
}

impl Packet for IngestionDataSharePacket {
    fn schema_raw() -> &'static str {
        INGESTION_DATA_SHARE_PACKET_SCHEMA
    }

    fn read<R: Read>(reader: &mut Reader<R>) -> Result<IngestionDataSharePacket, Error> {
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(Error::MalformedDataPacket(
                    "value is not a record".to_owned(),
                ))
            }
            Some(Err(e)) => {
                return Err(Error::Avro(
                    "failed to read record from Avro reader".to_owned(),
                    e,
                ));
            }
            None => return Err(Error::Eof),
        };

        // As in IngestionSignature::read_signature, , we can't just deserialize into a struct and
        // must instead walk the vector of record fields.
        let mut uuid = None;
        let mut encrypted_payload = None;
        let mut encryption_key_id = None;
        let mut r_pit = None;
        let mut version_configuration = None;
        let mut device_nonce = None;

        for tuple in record {
            match (tuple.0.as_str(), tuple.1) {
                ("uuid", Value::Uuid(v)) => uuid = Some(v),
                ("encrypted_payload", Value::Bytes(v)) => encrypted_payload = Some(v),
                ("encryption_key_id", Value::Union(boxed)) => match *boxed {
                    Value::String(v) => encryption_key_id = Some(v),
                    Value::Null => encryption_key_id = None,
                    v => {
                        return Err(Error::MalformedDataPacket(format!(
                            "unexpected boxed value {:?} in encryption_key_id",
                            v
                        )))
                    }
                },
                ("r_pit", Value::Long(v)) => r_pit = Some(v),
                ("version_configuration", Value::Union(boxed)) => match *boxed {
                    Value::String(v) => version_configuration = Some(v),
                    Value::Null => version_configuration = None,
                    v => {
                        return Err(Error::MalformedDataPacket(format!(
                            "unexpected boxed value {:?} in version_configuration",
                            v
                        )))
                    }
                },
                ("device_nonce", Value::Union(boxed)) => match *boxed {
                    Value::Bytes(v) => device_nonce = Some(v),
                    Value::Null => device_nonce = None,
                    v => {
                        return Err(Error::MalformedDataPacket(format!(
                            "unexpected boxed value {:?} in device_nonce",
                            v
                        )))
                    }
                },
                (f, _) => {
                    return Err(Error::MalformedDataPacket(format!(
                        "unexpected field {} in record",
                        f
                    )))
                }
            }
        }

        if uuid.is_none() || encrypted_payload.is_none() || r_pit.is_none() {
            return Err(Error::MalformedDataPacket(
                "missing fields in record".to_owned(),
            ));
        }

        Ok(IngestionDataSharePacket {
            uuid: uuid.unwrap(),
            encrypted_payload: encrypted_payload.unwrap(),
            encryption_key_id,
            r_pit: r_pit.unwrap(),
            version_configuration,
            device_nonce,
        })
    }

    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), Error> {
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
            .map_err(|e| Error::Avro("failed to append record to Avro writer".to_owned(), e))?;

        Ok(())
    }
}

/// The header on a Prio validation (sometimes referred to as verification)
/// batch.
#[derive(Debug, Deserialize, Serialize, PartialEq)]
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
    fn packet_file_digest(&self) -> &Vec<u8> {
        &self.packet_file_digest
    }

    fn read<R: Read>(reader: R) -> Result<ValidationHeader, Error> {
        let schema = Schema::parse_str(VALIDATION_HEADER_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse validation header schema".to_owned(), e))?;
        let mut reader = Reader::with_schema(&schema, reader).map_err(|e| {
            Error::Avro(
                "failed to create reader for validation header".to_owned(),
                e,
            )
        })?;

        // We expect exactly one record in the reader and for it to be an ingestion header
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => return Err(Error::MalformedHeader("value is not a record".to_owned())),
            Some(Err(e)) => {
                return Err(Error::Avro(
                    "failed to read header from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::Eof),
        };
        if reader.next().is_some() {
            return Err(Error::MalformedHeader("excess header in reader".to_owned()));
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
                            return Err(Error::MalformedHeader(format!(
                                "unexpected value {:?} for hamming weight",
                                v
                            )));
                        }
                    }
                }
                ("packet_file_digest", Value::Bytes(v)) => packet_file_digest = Some(v),
                (f, v) => {
                    return Err(Error::MalformedHeader(format!(
                        "unexpected field {} -> {:?} in record",
                        f, v
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
            return Err(Error::MalformedHeader(
                "missing field(s) in record".to_owned(),
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

    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(VALIDATION_HEADER_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse validation header schema".to_owned(), e))?;
        let mut writer = Writer::new(&schema, writer);

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
            .map_err(|e| Error::Avro("failed to append record to Avro writer".to_owned(), e))?;

        writer
            .flush()
            .map_err(|e| Error::Avro("failed to flush Avro writer".to_owned(), e))?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ValidationPacket {
    pub uuid: Uuid,
    pub f_r: i64,
    pub g_r: i64,
    pub h_r: i64,
}

impl Packet for ValidationPacket {
    fn schema_raw() -> &'static str {
        VALIDATION_PACKET_SCHEMA
    }

    fn read<R: Read>(reader: &mut Reader<R>) -> Result<ValidationPacket, Error> {
        let header = match reader.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => {
                return Err(Error::Avro(
                    "failed to read header from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::Eof),
        };

        from_value::<ValidationPacket>(&header)
            .map_err(|e| Error::Avro("failed to parse validation header".to_owned(), e))
    }

    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), Error> {
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
            .map_err(|e| Error::Avro("failed to append record to Avro writer".to_owned(), e))?;

        Ok(())
    }
}

impl TryFrom<&ValidationPacket> for VerificationMessage {
    type Error = TryFromIntError;

    fn try_from(p: &ValidationPacket) -> Result<Self, Self::Error> {
        Ok(VerificationMessage {
            f_r: Field::from(u32::try_from(p.f_r)?),
            g_r: Field::from(u32::try_from(p.g_r)?),
            h_r: Field::from(u32::try_from(p.h_r)?),
        })
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
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
    pub fn sum(&self) -> Result<Vec<Field>, TryFromIntError> {
        self.sum
            .iter()
            .map(|i| Ok(Field::from(u32::try_from(*i)?)))
            .collect::<Result<Vec<_>, _>>()
    }
}

impl Header for SumPart {
    fn packet_file_digest(&self) -> &Vec<u8> {
        &self.packet_file_digest
    }

    fn read<R: Read>(reader: R) -> Result<SumPart, Error> {
        let schema = Schema::parse_str(SUM_PART_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse sum part schema".to_owned(), e))?;
        let mut reader = Reader::with_schema(&schema, reader)
            .map_err(|e| Error::Avro("failed to create reader for sum part".to_owned(), e))?;

        // We expect exactly one record in the reader and for it to be a sum
        // part.
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => return Err(Error::MalformedHeader("value is not a record".to_owned())),
            Some(Err(e)) => {
                return Err(Error::Avro(
                    "failed to read header from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::Eof),
        };
        if reader.next().is_some() {
            return Err(Error::MalformedHeader("excess header in reader".to_owned()));
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
                                    Err(Error::MalformedHeader(format!(
                                        "unexpected value in batch_uuids array {:?}",
                                        value
                                    )))
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
                            return Err(Error::MalformedHeader(format!(
                                "unexpected value {:?} for hamming weight",
                                v
                            )));
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
                                    Err(Error::MalformedHeader(format!(
                                        "unexpected value in sum array {:?}",
                                        value
                                    )))
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
                    return Err(Error::MalformedHeader(format!(
                        "unexpected field {} -> {:?} in record",
                        f, v
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
            return Err(Error::MalformedHeader(
                "missing field(s) in record".to_owned(),
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

    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(SUM_PART_SCHEMA)
            .map_err(|e| Error::Avro("failed to parse sum part schema".to_owned(), e))?;
        let mut writer = Writer::new(&schema, writer);

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
            .map_err(|e| Error::Avro("failed to append record to Avro writer".to_owned(), e))?;

        writer
            .flush()
            .map_err(|e| Error::Avro("failed to flush Avro writer".to_owned(), e))?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct InvalidPacket {
    pub uuid: Uuid,
}

impl Packet for InvalidPacket {
    fn schema_raw() -> &'static str {
        INVALID_PACKET_SCHEMA
    }

    fn read<R: Read>(reader: &mut Reader<R>) -> Result<InvalidPacket, Error> {
        let header = match reader.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => {
                return Err(Error::Avro(
                    "failed to read invalid packet from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::Eof),
        };

        from_value::<InvalidPacket>(&header)
            .map_err(|e| Error::Avro("failed to parse invalid packet".to_owned(), e))
    }

    fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), Error> {
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
            .map_err(|e| Error::Avro("failed to append record to Avro writer".to_owned(), e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_batch_signature() {
        let signature1 = BatchSignature {
            batch_header_signature: vec![1u8, 2u8, 3u8, 4u8],
            key_identifier: "my-cool-key".to_owned(),
        };
        let signature2 = BatchSignature {
            batch_header_signature: vec![5u8, 6u8, 7u8, 9u8],
            key_identifier: "my-other-key".to_owned(),
        };

        let mut record_vec = Vec::new();

        signature1.write(&mut record_vec).unwrap();
        let signature_again = BatchSignature::read(&record_vec[..]).unwrap();
        assert_eq!(signature1, signature_again);
        assert!(signature2 != signature_again);
    }

    #[test]
    fn roundtrip_ingestion_header() {
        let headers = &[
            IngestionHeader {
                batch_uuid: Uuid::new_v4(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: None,
                batch_start_time: 789456123,
                batch_end_time: 789456321,
                packet_file_digest: vec![1u8],
            },
            IngestionHeader {
                batch_uuid: Uuid::new_v4(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: Some(12),
                batch_start_time: 789456123,
                batch_end_time: 789456321,
                packet_file_digest: vec![2u8],
            },
        ];

        for header in headers {
            let mut record_vec = Vec::new();

            header.write(&mut record_vec).expect("write error");
            let header_again = IngestionHeader::read(&record_vec[..]).expect("read error");
            assert_eq!(header_again, *header);
        }
    }

    #[test]
    fn roundtrip_data_share_packet() {
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

        let mut record_vec = Vec::new();

        let schema = IngestionDataSharePacket::schema();
        let mut writer = Writer::new(&schema, &mut record_vec);

        for packet in packets {
            packet.write(&mut writer).expect("write error");
        }
        writer.flush().unwrap();

        let mut reader = Reader::with_schema(&schema, &record_vec[..]).unwrap();
        for packet in packets {
            let packet_again = IngestionDataSharePacket::read(&mut reader).expect("read error");
            assert_eq!(packet_again, *packet);
        }

        // Do one more read. This should yield EOF.
        match IngestionDataSharePacket::read(&mut reader) {
            Err(Error::Eof) => (),
            v => assert!(false, "wrong error {:?}", v),
        }
    }

    #[test]
    fn roundtrip_validation_header() {
        let headers = &[
            ValidationHeader {
                batch_uuid: Uuid::new_v4(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: None,
                packet_file_digest: vec![4u8],
            },
            ValidationHeader {
                batch_uuid: Uuid::new_v4(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: Some(12),
                packet_file_digest: vec![6u8],
            },
        ];

        for header in headers {
            let mut record_vec = Vec::new();

            header.write(&mut record_vec).expect("write error");
            let header_again = ValidationHeader::read(&record_vec[..]).expect("read error");
            assert_eq!(header_again, *header);
        }
    }

    #[test]
    fn roundtrip_validation_packet() {
        let packets = &[
            ValidationPacket {
                uuid: Uuid::new_v4(),
                f_r: 1,
                g_r: 2,
                h_r: 3,
            },
            ValidationPacket {
                uuid: Uuid::new_v4(),
                f_r: 4,
                g_r: 5,
                h_r: 6,
            },
            ValidationPacket {
                uuid: Uuid::new_v4(),
                f_r: 7,
                g_r: 8,
                h_r: 9,
            },
        ];

        let mut record_vec = Vec::new();

        let schema = ValidationPacket::schema();
        let mut writer = Writer::new(&schema, &mut record_vec);

        for packet in packets {
            packet.write(&mut writer).expect("write error");
        }
        writer.flush().unwrap();

        let mut reader = Reader::with_schema(&schema, &record_vec[..]).unwrap();
        for packet in packets {
            let packet_again = ValidationPacket::read(&mut reader).expect("read error");
            assert_eq!(packet_again, *packet);
        }

        // Do one more read. This should yield EOF.
        match ValidationPacket::read(&mut reader) {
            Err(Error::Eof) => (),
            v => assert!(false, "wrong error {:?}", v),
        }
    }

    #[test]
    fn roundtrip_sum_part() {
        let headers = &[
            SumPart {
                batch_uuids: vec![Uuid::new_v4(), Uuid::new_v4()],
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
            },
            SumPart {
                batch_uuids: vec![Uuid::new_v4()],
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
            },
        ];

        for header in headers {
            let mut record_vec = Vec::new();

            header.write(&mut record_vec).expect("write error");
            let header_again = SumPart::read(&record_vec[..]).expect("read error");
            assert_eq!(header_again, *header);
        }
    }

    #[test]
    fn roundtrip_invalid_packet() {
        let packets = &[
            InvalidPacket {
                uuid: Uuid::new_v4(),
            },
            InvalidPacket {
                uuid: Uuid::new_v4(),
            },
            InvalidPacket {
                uuid: Uuid::new_v4(),
            },
        ];

        let mut record_vec = Vec::new();

        let schema = InvalidPacket::schema();
        let mut writer = Writer::new(&schema, &mut record_vec);

        for packet in packets {
            packet.write(&mut writer).expect("write error");
        }
        writer.flush().unwrap();

        let mut reader = Reader::with_schema(&schema, &record_vec[..]).unwrap();
        for packet in packets {
            let packet_again = InvalidPacket::read(&mut reader).expect("read error");
            assert_eq!(packet_again, *packet);
        }

        // Do one more read. This should yield EOF.
        match InvalidPacket::read(&mut reader) {
            Err(Error::Eof) => (),
            v => assert!(false, "wrong error {:?}", v),
        }
    }
}
