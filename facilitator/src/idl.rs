use crate::Error;
use avro_rs::types::{Record, Value};
use avro_rs::{from_value, Reader, Schema, Writer};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use uuid::Uuid;

const INGESTION_HEADER_SCHEMA: &str = r#"
{
    "namespace": "org.abetterinternet.prio.v1",
    "type": "record",
    "name": "PrioBatchHeader",
    "fields": [
        {
            "name": "batch_uuid",
            "type": "string",
            "logicalType": "uuid",
            "doc": "Universal unique identifier to link with data share batch sent to other server(s) participating in the aggregation."
        },
        {
            "name": "name",
            "type": "string",
            "doc": "a name for this specific aggregation"
        },
        {
            "name": "bins",
            "type": "int",
            "doc": "number of bins for this aggregation"
        },
        {
            "name": "epsilon",
            "type": "double",
            "doc": "differential privacy parameter for local randomization before aggregation."
        },
        {
            "name": "prime",
            "type": "long",
            "default": 4293918721,
            "doc": "the value of prime p used in aggregation."
        },
        {
            "name": "number_of_servers",
            "type": "int",
            "default": 2,
            "doc": "the number of servers that will be involved in the aggregation."
        },
        {
            "name": "hamming_weight",
            "type": ["int", "null"],
            "doc":  "If specified, the hamming weight of the vector will be verified during the validity check on the server."
        },
        {
            "name": "batch_start_time",
            "type": "long",
            "logicalType": "timestamp-millis",
            "doc": "time range information for the shares in this batch."
        },
        {
            "name": "batch_end_time",
            "type": "long",
            "logicalType": "timestamp-millis",
            "doc": "time range information for the shares in this batch."
        }
    ]
}
"#;

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
}

impl IngestionHeader {
    /// Reads and parses one IngestionHeader from the provided std::io::Read
    /// instance.
    pub fn read<R: Read>(reader: R) -> Result<IngestionHeader, Error> {
        let schema = Schema::parse_str(INGESTION_HEADER_SCHEMA).map_err(|e| {
            Error::AvroError("failed to parse ingestion header schema".to_owned(), e)
        })?;
        let mut reader = Reader::with_schema(&schema, reader).map_err(|e| {
            Error::AvroError("failed to create reader for ingestion header".to_owned(), e)
        })?;

        // We expect exactly one record in the reader and for it to be an ingestion header
        let header = match reader.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => {
                return Err(Error::AvroError(
                    "failed to read header from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::EofError),
        };
        if let Some(_) = reader.next() {
            return Err(Error::MalformedHeaderError(
                "excess header in reader".to_owned(),
            ));
        }

        from_value::<IngestionHeader>(&header)
            .map_err(|e| Error::AvroError("failed to parse ingestion header".to_owned(), e))
    }

    /// Serializes this header into Avro format and writes it to the provided
    /// std::io::Write instance.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(INGESTION_HEADER_SCHEMA).map_err(|e| {
            Error::AvroError("failed to parse ingestion header schema".to_owned(), e)
        })?;
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

        if let Err(e) = writer.append(record) {
            return Err(Error::AvroError(
                "failed to append record to Avro writer".to_owned(),
                e,
            ));
        }

        if let Err(e) = writer.flush() {
            return Err(Error::AvroError(
                "failed to flush Avro writer".to_owned(),
                e,
            ));
        }
        Ok(())
    }
}

// TODO We ought to use the same signature format across ingestion, validation
// and accumulation, so this should not be ingestion-specific.
const INGESTION_SIGNATURE_SCHEMA: &str = r#"
{
    "namespace": "org.abetterinternet.prio.v1",
    "type": "record",
    "name": "PrioIngestionSignature",
    "fields": [
        {
            "name": "batch_header_signature",
            "type": "bytes",
            "doc": "signature of the PrioBatchHeader object"
        },
        {
            "name": "signature_of_packets",
            "type": "bytes",
            "doc": "signature of the avro file of individual shares in this batch."
        }
    ]
}
"#;

/// The file containing signatures over the ingestion batch header and packet
/// file.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct IngestionSignature {
    pub batch_header_signature: Vec<u8>,
    pub signature_of_packets: Vec<u8>,
}

impl IngestionSignature {
    /// Reads and parses one IngestionSignature from the provided std::io::Read
    /// instance.
    pub fn read<R: Read>(reader: R) -> Result<IngestionSignature, Error> {
        let schema = Schema::parse_str(INGESTION_SIGNATURE_SCHEMA).map_err(|e| {
            Error::AvroError("failed to parse ingestion signature schema".to_owned(), e)
        })?;
        let mut reader = Reader::with_schema(&schema, reader)
            .map_err(|e| Error::AvroError("failed to create Avro reader".to_owned(), e))?;

        // We expect exactly one record and for it to be an ingestion signature
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(Error::MalformedSignatureError(
                    "value is not a record".to_owned(),
                ))
            }
            Some(Err(e)) => {
                return Err(Error::AvroError(
                    "failed to read record from Avro reader".to_owned(),
                    e,
                ));
            }
            None => return Err(Error::EofError),
        };
        if let Some(_) = reader.next() {
            return Err(Error::MalformedSignatureError(
                "excess value in reader".to_owned(),
            ));
        }

        // Here we might wish to use from_value::<IngestionSignature>(record) but avro_rs does not
        // seem to recognize it as a Bytes and fails to deserialize it. The value we unwrapped from
        // reader.next above is a vector of (String, avro_rs::Value) tuples, which we now iterate to
        // find the struct members.
        let mut batch_header_signature = None;
        let mut signature_of_packets = None;

        for tuple in record {
            match (tuple.0.as_str(), tuple.1) {
                ("batch_header_signature", Value::Bytes(v)) => batch_header_signature = Some(v),
                ("signature_of_packets", Value::Bytes(v)) => signature_of_packets = Some(v),
                (f, _) => {
                    return Err(Error::MalformedSignatureError(format!(
                        "unexpected field {} in record",
                        f
                    )))
                }
            }
        }

        if batch_header_signature.is_none() || signature_of_packets.is_none() {
            return Err(Error::MalformedSignatureError(
                "missing fields in record".to_owned(),
            ));
        }

        Ok(IngestionSignature {
            batch_header_signature: batch_header_signature.unwrap(),
            signature_of_packets: signature_of_packets.unwrap(),
        })
    }

    /// Serializes this signature into Avro format and writes it to the provided
    /// std::io::Write instance.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(INGESTION_SIGNATURE_SCHEMA).map_err(|e| {
            Error::AvroError("failed to parse ingestion signature schema".to_owned(), e)
        })?;
        let mut writer = Writer::new(&schema, writer);

        let mut record = match Record::new(writer.schema()) {
            Some(r) => r,
            None => {
                // avro_rs docs say this can only happen "if the `Schema is not a `Schema::Record`
                // variant", which shouldn't ever happen, so panic for debugging
                // https://docs.rs/avro-rs/0.11.0/avro_rs/types/struct.Record.html#method.new
                panic!("Unable to create Record from ingestion signature schema");
            }
        };

        record.put(
            "batch_header_signature",
            Value::Bytes(self.batch_header_signature.clone()),
        );
        record.put(
            "signature_of_packets",
            Value::Bytes(self.signature_of_packets.clone()),
        );
        if let Err(e) = writer.append(record) {
            return Err(Error::AvroError(
                "failed to append record to Avro writer".to_owned(),
                e,
            ));
        }

        if let Err(e) = writer.flush() {
            return Err(Error::AvroError(
                "failed to flush Avro writer".to_owned(),
                e,
            ));
        }
        Ok(())
    }
}

const INGESTION_DATA_SHARE_PACKET_SCHEMA: &str = r#"
{
    "namespace": "org.abetterinternet.prio.v1",
    "type": "record",
    "name": "PrioDataSharePacket",
    "fields": [
        {
            "name": "uuid",
            "type": "string",
            "logicalType": "uuid",
            "doc": "Universal unique identifier to link with data share sent to other server(s) participating in the aggregation."
        },
        {
            "name": "encrypted_payload",
            "type": "bytes",
            "doc": "The encrypted content of the data share algorithm. This represents one of the Vec<u8> results from https://github.com/abetterinternet/libprio-rs/blob/f0092de421c70de9888cfcbbc86be7b5c5e624b0/src/client.rs#L49"
        },
        {
            "name": "encryption_key_id",
            "type": "string",
            "doc": "Encryption key identifier (e.g., to support key rotations)"
        },
        {
            "name": "r_pit",
            "type": "long",
            "doc": "The random value r_PIT to use for the Polynomial Identity Test."
        },
        {
            "name": "version_configuration",
            "type": [
                "null",
                "string"
            ],
            "doc": "Version configuration of the device."
        },
        {
            "name": "device_nonce",
            "type": [
                "null",
                "bytes"
            ],
            "doc": "SHA256 hash of the BAA certificate issued to the client device. This would be populated only in cases where ingestion cannot fully address spam/abuse."
        }
    ]
}
"#;

/// Creates an avro_rs::Schema from the ingestion data share packet schema. For
/// use with IngestionDataSharePacket::{read, write}. Since this only ever uses
/// a schema we own, it panics on failure.
pub fn ingestion_data_share_packet_schema() -> Schema {
    Schema::parse_str(INGESTION_DATA_SHARE_PACKET_SCHEMA).unwrap()
}

/// A single packet from an ingestion batch file. Note that unlike the header
/// and signature, which are files containing a single record, the data share
/// file will contain many IngestionDataSharePacket records.
#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct IngestionDataSharePacket {
    pub uuid: Uuid,
    pub encrypted_payload: Vec<u8>,
    pub encryption_key_id: String,
    pub r_pit: i64,
    pub version_configuration: Option<String>,
    pub device_nonce: Option<Vec<u8>>,
}

impl IngestionDataSharePacket {
    /// Reads and parses a single IngestionDataSharePacket from the provided
    /// avro_rs::Reader. Note that unlike other structures, this does not take
    /// a primitive std::io::Read, because we do not want to create a new Avro
    /// schema and reader for each packet. The Reader must have been created
    /// with the schema returned from ingestion_data_share_packet_schema.
    pub fn read<R: Read>(reader: &mut Reader<R>) -> Result<IngestionDataSharePacket, Error> {
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(Error::MalformedDataPacketError(
                    "value is not a record".to_owned(),
                ))
            }
            Some(Err(e)) => {
                return Err(Error::AvroError(
                    "failed to read record from Avro reader".to_owned(),
                    e,
                ));
            }
            None => return Err(Error::EofError),
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
                ("encryption_key_id", Value::String(v)) => encryption_key_id = Some(v),
                ("r_pit", Value::Long(v)) => r_pit = Some(v),
                ("version_configuration", Value::Union(boxed)) => match *boxed {
                    Value::String(v) => version_configuration = Some(v),
                    Value::Null => version_configuration = None,
                    v => {
                        return Err(Error::MalformedDataPacketError(format!(
                            "unexpected boxed value {:?} in version_configuration",
                            v
                        )))
                    }
                },
                ("device_nonce", Value::Union(boxed)) => match *boxed {
                    Value::Bytes(v) => device_nonce = Some(v),
                    Value::Null => device_nonce = None,
                    v => {
                        return Err(Error::MalformedDataPacketError(format!(
                            "unexpected boxed value {:?} in device_nonce",
                            v
                        )))
                    }
                },
                (f, _) => {
                    return Err(Error::MalformedDataPacketError(format!(
                        "unexpected field {} in record",
                        f
                    )))
                }
            }
        }

        if uuid.is_none()
            || encrypted_payload.is_none()
            || encryption_key_id.is_none()
            || r_pit.is_none()
        {
            return Err(Error::MalformedDataPacketError(
                "missing fields in record".to_owned(),
            ));
        }

        Ok(IngestionDataSharePacket {
            uuid: uuid.unwrap(),
            encrypted_payload: encrypted_payload.unwrap(),
            encryption_key_id: encryption_key_id.unwrap(),
            r_pit: r_pit.unwrap(),
            version_configuration: version_configuration,
            device_nonce: device_nonce,
        })
    }

    /// Serializes and writes a single IngestionDataSharePacket to the provided
    /// avro_rs::Writer. Note that unlike other structures, this does not take
    /// a primitive std::io::Write, because we do not want to create a new Avro
    /// schema and reader for each packet. The Reader must have been created
    /// with the schema returned from ingestion_data_share_packet_schema.
    pub fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), Error> {
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
        record.put(
            "encrypted_payload",
            Value::Bytes(self.encrypted_payload.clone()),
        );
        record.put(
            "encryption_key_id",
            Value::String(self.encryption_key_id.clone()),
        );
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

        if let Err(e) = writer.append(record) {
            return Err(Error::AvroError("failed to append record".to_owned(), e));
        }

        Ok(())
    }
}

const VALIDATION_HEADER_SCHEMA: &str = r#"
{
    "namespace": "org.abetterinternet.prio.v1",
    "type": "record",
    "name": "PrioValidityHeader",
    "fields": [
        {
            "name": "batch_uuid",
            "type": "string",
            "logicalType": "uuid",
            "doc": "Universal unique identifier to link with data share batch sent to other server(s) participating in the aggregation."
        },
        {
            "name": "name",
            "type": "string",
            "doc": "a name for this specific aggregation"
        },
        {
            "name": "bins",
            "type": "int",
            "doc": "number of bins for this aggregation"
        },
        {
            "name": "epsilon",
            "type": "double",
            "doc": "differential privacy parameter for local randomization before aggregation."
        },
        {
            "name": "prime",
            "type": "long",
            "default": 4293918721,
            "doc": "the value of prime p used in aggregation."
        },
        {
            "name": "number_of_servers",
            "type": "int",
            "default": 2,
            "doc": "the number of servers that will be involved in the aggregation."
        },
        {
            "name": "hamming_weight",
            "type": [
                "int",
                "null"
            ],
            "doc": "If specified, the hamming weight of the vector will be verified during the validity check on the server."
        }
    ]
}
"#;

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
}

impl ValidationHeader {
    /// Reads and parses one ValidationHeader from the provided std::io::Read
    /// instance.
    pub fn read<R: Read>(reader: R) -> Result<ValidationHeader, Error> {
        let schema = Schema::parse_str(VALIDATION_HEADER_SCHEMA).map_err(|e| {
            Error::AvroError("failed to parse validation header schema".to_owned(), e)
        })?;
        let mut reader = Reader::with_schema(&schema, reader).map_err(|e| {
            Error::AvroError(
                "failed to create reader for validation header".to_owned(),
                e,
            )
        })?;

        let header = match reader.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => {
                return Err(Error::AvroError(
                    "failed to read header from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::EofError),
        };
        if let Some(_) = reader.next() {
            return Err(Error::MalformedHeaderError(
                "excess header in reader".to_owned(),
            ));
        }

        from_value::<ValidationHeader>(&header)
            .map_err(|e| Error::AvroError("failed to parse validation header".to_owned(), e))
    }

    /// Serializes this header into Avro format and writes it to the provided
    /// std::io::Write instance.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(VALIDATION_HEADER_SCHEMA).map_err(|e| {
            Error::AvroError("failed to parse validation header schema".to_owned(), e)
        })?;
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
        match self.hamming_weight {
            Some(v) => record.put("hamming_weight", Value::Union(Box::new(Value::Int(v)))),
            None => record.put("hamming_weight", Value::Union(Box::new(Value::Null))),
        }

        if let Err(e) = writer.append(record) {
            return Err(Error::AvroError(
                "failed to append record to Avro writer".to_owned(),
                e,
            ));
        }

        if let Err(e) = writer.flush() {
            return Err(Error::AvroError(
                "failed to flush Avro writer".to_owned(),
                e,
            ));
        }
        Ok(())
    }
}

const VALIDATION_PACKET_SCHEMA: &str = r#"
{
    "namespace": "org.abetterinternet.prio.v1",
    "type": "record",
    "name": "PrioValidityPacket",
    "fields": [
        {
            "name": "uuid",
            "type": "string",
            "logicalType": "uuid",
            "doc": "Universal unique identifier to link with data share sent to other server(s) participating in the aggregation."
        },
        {
            "name": "f_r",
            "type": "long",
            "doc": "The share of the polynomial f evaluated in r_PIT."
        },
        {
            "name": "g_r",
            "type": "long",
            "doc": "The share of the polynomial g evaluated in r_PIT."
        },
        {
            "name": "h_r",
            "type": "long",
            "doc": "The share of the polynomial h evaluated in r_PIT."
        }
    ]
}
"#;

/// Creates an avro_rs::Schema from the validation packet schema. For use with
/// ValidationPacket::{read, write}. Since this can only use a schema we own,
/// panics on failure.
pub fn validation_packet_schema() -> Schema {
    Schema::parse_str(VALIDATION_PACKET_SCHEMA).unwrap()
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ValidationPacket {
    pub uuid: Uuid,
    pub f_r: i64,
    pub g_r: i64,
    pub h_r: i64,
}

impl ValidationPacket {
    pub fn read<R: Read>(reader: &mut Reader<R>) -> Result<ValidationPacket, Error> {
        let header = match reader.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => {
                return Err(Error::AvroError(
                    "failed to read header from Avro reader".to_owned(),
                    e,
                ))
            }
            None => return Err(Error::EofError),
        };

        from_value::<ValidationPacket>(&header)
            .map_err(|e| Error::AvroError("failed to parse validation header".to_owned(), e))
    }

    pub fn write<W: Write>(&self, writer: &mut Writer<W>) -> Result<(), Error> {
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

        if let Err(e) = writer.append(record) {
            return Err(Error::AvroError("failed to append record".to_owned(), e));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            },
        ];

        for header in headers {
            let mut record_vec = Vec::new();

            let res = header.write(&mut record_vec);
            assert!(res.is_ok(), "write error {:?}", res);
            let header_again = IngestionHeader::read(&record_vec[..]);
            assert!(header_again.is_ok(), "read error {:?}", header_again);

            assert_eq!(header_again.unwrap(), *header);
        }
    }

    #[test]
    fn roundtrip_signature() {
        let signature = IngestionSignature {
            batch_header_signature: vec![0u8, 1u8, 2u8, 3u8],
            signature_of_packets: vec![4u8, 5u8, 6u8, 7u8],
        };

        let mut record_vec = Vec::new();
        let res = signature.write(&mut record_vec);
        assert!(res.is_ok(), "write error {:?}", res);
        let signature_again = IngestionSignature::read(&record_vec[..]);
        assert!(signature_again.is_ok(), "read error {:?}", signature_again);
        assert_eq!(signature_again.unwrap(), signature);
    }

    #[test]
    fn roundtrip_data_share_packet() {
        let packets = &[
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![0u8, 1u8, 2u8, 3u8],
                encryption_key_id: "fake-key-1".to_owned(),
                r_pit: 1,
                version_configuration: Some("config-1".to_owned()),
                device_nonce: None,
            },
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![4u8, 5u8, 6u8, 7u8],
                encryption_key_id: "fake-key-2".to_owned(),
                r_pit: 2,
                version_configuration: None,
                device_nonce: Some(vec![8u8, 9u8, 10u8, 11u8]),
            },
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![8u8, 9u8, 10u8, 11u8],
                encryption_key_id: "fake-key-3".to_owned(),
                r_pit: 3,
                version_configuration: None,
                device_nonce: None,
            },
        ];

        let mut record_vec = Vec::new();

        let schema = ingestion_data_share_packet_schema();
        let mut writer = Writer::new(&schema, &mut record_vec);

        for packet in packets {
            let res = packet.write(&mut writer);
            assert!(res.is_ok(), "write error {:?}", res);
        }
        writer.flush().unwrap();

        let mut reader = Reader::with_schema(&schema, &record_vec[..]).unwrap();
        for packet in packets {
            let packet_again = IngestionDataSharePacket::read(&mut reader);
            assert!(packet_again.is_ok(), "read error {:?}", packet_again);
            assert_eq!(packet_again.unwrap(), *packet);
        }

        // Do one more read. This should yield EOF.
        match IngestionDataSharePacket::read(&mut reader) {
            Err(Error::EofError) => (),
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
            },
            ValidationHeader {
                batch_uuid: Uuid::new_v4(),
                name: "fake-batch".to_owned(),
                bins: 2,
                epsilon: 1.601,
                prime: 17,
                number_of_servers: 2,
                hamming_weight: Some(12),
            },
        ];

        for header in headers {
            let mut record_vec = Vec::new();

            let res = header.write(&mut record_vec);
            assert!(res.is_ok(), "write error {:?}", res);
            let header_again = ValidationHeader::read(&record_vec[..]);
            assert!(header_again.is_ok(), "read error {:?}", header_again);

            assert_eq!(header_again.unwrap(), *header);
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

        let schema = validation_packet_schema();
        let mut writer = Writer::new(&schema, &mut record_vec);

        for packet in packets {
            let res = packet.write(&mut writer);
            assert!(res.is_ok(), "write error {:?}", res);
        }
        writer.flush().unwrap();

        let mut reader = Reader::with_schema(&schema, &record_vec[..]).unwrap();
        for packet in packets {
            let packet_again = ValidationPacket::read(&mut reader);
            assert!(packet_again.is_ok(), "read error {:?}", packet_again);
            assert_eq!(packet_again.unwrap(), *packet);
        }

        // Do one more read. This should yield EOF.
        match ValidationPacket::read(&mut reader) {
            Err(Error::EofError) => (),
            v => assert!(false, "wrong error {:?}", v),
        }
    }
}
