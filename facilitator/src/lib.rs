use libprio_rs::encrypt::EncryptError;

pub mod idl;
pub mod ingestion;
pub mod sample;
pub mod transport;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";

/// Default keys used in testing and for sample data generation. These are
/// stored in base64 to make it convenient to copy/paste them into other tools
/// or programs that may wish to consume sample data emitted by this program
/// with these keys.
pub const DEFAULT_PHA_PRIVATE_KEY: &str =
    "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9Rq\
    Zx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==";
pub const DEFAULT_FACILITATOR_PRIVATE_KEY: &str =
    "BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g\
    /WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w==";
pub const DEFAULT_INGESTOR_PRIVATE_KEY: &str =
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5b\
    WIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxo\
    uP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs";

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
    CryptographyError(
        String,
        Option<ring::error::KeyRejected>,
        Option<ring::error::Unspecified>,
    ),
}

/// Constructs an EcdsaKeyPair from the default ingestor server. Should only be
/// used in tests, and since we know DEFAULT_INGESTOR_PRIVATE_KEY is valid, it
/// is ok to unwrap() here.
pub fn default_ingestor_private_key() -> Vec<u8> {
    base64::decode(DEFAULT_INGESTOR_PRIVATE_KEY).unwrap()
}
