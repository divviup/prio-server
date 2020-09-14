use libprio_rs::encrypt::EncryptError;

pub mod idl;
pub mod ingestion;
pub mod sample;
pub mod transport;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";
pub const DEFAULT_PHA_PRIVATE_KEY: &str =
    "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9Rq\
    Zx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==";
pub const DEFAULT_FACILITATOR_PRIVATE_KEY: &str =
    "BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g\
    /WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w==";

#[derive(Debug)]
pub enum Error {
    AvroError(String, avro_rs::Error),
    MalformedHeaderError(String),
    MalformedSignatureError(String),
    MalformedDataPacketError(String),
    EofError,
    IoError(String, std::io::Error),
    LibPrioError(String, Option<EncryptError>),
    FacilitatorError(String),
    IllegalArgumentError(String),
}
