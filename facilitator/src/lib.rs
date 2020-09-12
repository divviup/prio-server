pub mod idl;
pub mod transport;

#[derive(Debug)]
pub enum Error {
    AvroError(avro_rs::Error),
    MalformedHeaderError(String),
    MalformedSignatureError(String),
    MalformedDataPacketError(String),
    IoError(std::io::Error),
}
