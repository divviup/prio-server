use libprio_rs::encrypt::EncryptError;

pub mod idl;
pub mod sample;
pub mod transport;

#[derive(Debug)]
pub enum Error {
    AvroError(String, avro_rs::Error),
    MalformedHeaderError(String),
    MalformedSignatureError(String),
    MalformedDataPacketError(String),
    IoError(String, std::io::Error),
    LibPrioError(String, Option<EncryptError>),
    FacilitatorError(String),
    IllegalArgumentError(String),
}
