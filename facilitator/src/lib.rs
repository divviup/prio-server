pub mod idl;

#[derive(Debug)]
pub enum Error {
    AvroError(avro_rs::Error),
    MalformedHeaderError(String),
    MalformedSignatureError(String),
    MalformedDataPacketError(String),
}
