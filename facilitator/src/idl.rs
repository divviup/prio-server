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
    pub fn read(reader: impl Read) -> Result<IngestionHeader, Error> {
        let schema = Schema::parse_str(INGESTION_HEADER_SCHEMA).map_err(|e| Error::AvroError(e))?;
        let mut reader = Reader::with_schema(&schema, reader).map_err(|e| Error::AvroError(e))?;

        // We expect exactly one record in the reader and for it to be an ingestion header
        let header = match reader.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => return Err(Error::AvroError(e)),
            None => {
                return Err(Error::MalformedHeaderError(
                    "no records found in reader".to_owned(),
                ));
            }
        };
        if let Some(_) = reader.next() {
            return Err(Error::MalformedHeaderError(
                "excess header in reader".to_owned(),
            ));
        }

        from_value::<IngestionHeader>(&header).map_err(|e| Error::AvroError(e))
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(INGESTION_HEADER_SCHEMA).map_err(|e| Error::AvroError(e))?;
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
            return Err(Error::AvroError(e));
        }

        if let Err(e) = writer.flush() {
            return Err(Error::AvroError(e));
        }
        Ok(())
    }
}

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

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct IngestionSignature {
    pub batch_header_signature: Vec<u8>,
    pub signature_of_packets: Vec<u8>,
}

impl IngestionSignature {
    pub fn read(reader: impl Read) -> Result<IngestionSignature, Error> {
        let schema =
            Schema::parse_str(INGESTION_SIGNATURE_SCHEMA).map_err(|e| Error::AvroError(e))?;
        let mut reader = Reader::with_schema(&schema, reader).map_err(|e| Error::AvroError(e))?;

        // We expect exactly one record and for it to be an ingestion signature
        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(Error::MalformedSignatureError(
                    "value is not a record".to_owned(),
                ))
            }
            Some(Err(e)) => {
                return Err(Error::AvroError(e));
            }
            None => {
                return Err(Error::MalformedSignatureError(
                    "no value from reader".to_owned(),
                ));
            }
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

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema =
            Schema::parse_str(INGESTION_SIGNATURE_SCHEMA).map_err(|e| Error::AvroError(e))?;
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
            return Err(Error::AvroError(e));
        }

        if let Err(e) = writer.flush() {
            return Err(Error::AvroError(e));
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
    pub fn read(reader: impl Read) -> Result<IngestionDataSharePacket, Error> {
        let schema = Schema::parse_str(INGESTION_DATA_SHARE_PACKET_SCHEMA)
            .map_err(|e| Error::AvroError(e))?;
        // TODO do we want to create an Avro Reader each time we read a packet? Might work provided
        // we advance through the impl Read appropriately
        let mut reader = Reader::with_schema(&schema, reader).map_err(|e| Error::AvroError(e))?;

        let record = match reader.next() {
            Some(Ok(Value::Record(r))) => r,
            Some(Ok(_)) => {
                return Err(Error::MalformedDataPacketError(
                    "value is not a record".to_owned(),
                ))
            }
            Some(Err(e)) => {
                return Err(Error::AvroError(e));
            }
            None => {
                return Err(Error::MalformedDataPacketError(
                    "no value from reader".to_owned(),
                ));
            }
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
                        return Err(Error::MalformedSignatureError(format!(
                            "unexpected boxed value {:?} in version_configuration",
                            v
                        )))
                    }
                },
                ("device_nonce", Value::Union(boxed)) => match *boxed {
                    Value::Bytes(v) => device_nonce = Some(v),
                    Value::Null => device_nonce = None,
                    v => {
                        return Err(Error::MalformedSignatureError(format!(
                            "unexpected boxed value {:?} in device_nonce",
                            v
                        )))
                    }
                },
                (f, _) => {
                    return Err(Error::MalformedSignatureError(format!(
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
            return Err(Error::MalformedSignatureError(
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

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let schema = Schema::parse_str(INGESTION_DATA_SHARE_PACKET_SCHEMA)
            .map_err(|e| Error::AvroError(e))?;
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
            return Err(Error::AvroError(e));
        }

        if let Err(e) = writer.flush() {
            return Err(Error::AvroError(e));
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
        ];

        for packet in packets {
            let mut record_vec = Vec::new();

            let res = packet.write(&mut record_vec);
            assert!(res.is_ok(), "write error {:?}", res);
            let packet_again = IngestionDataSharePacket::read(&record_vec[..]);
            assert!(packet_again.is_ok(), "read error {:?}", packet_again);
            assert_eq!(packet_again.unwrap(), *packet);
        }
    }
}
