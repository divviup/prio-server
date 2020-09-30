use crate::Error;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED,
    ECDSA_P256_SHA256_FIXED_SIGNING,
};

/// Default keys used in testing and for sample data generation. These are
/// stored in base64 to make it convenient to copy/paste them into other tools
/// or programs that may wish to consume sample data emitted by this program
/// with these keys.
pub const DEFAULT_PHA_ECIES_PRIVATE_KEY: &str =
    "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9Rq\
    Zx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==";
pub const DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY: &str =
    "BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g\
    /WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w==";
pub const DEFAULT_INGESTOR_PRIVATE_KEY: &str =
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5b\
    WIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxo\
    uP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs";
pub const DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY: &str =
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeSa+S+tmLupnAEyFK\
    dVuKB99y09YEqW41+8pwP4cTkahRANCAASy7FHcLGnRudVHWga/j2k9nQ3lMvuGE01\
    Q7DEyjyCuuw9YmB3dHvYcRUnxVRI/nF5LvneGim0dC7F1fuRAPeXI";
pub const DEFAULT_PHA_SIGNING_PRIVATE_KEY: &str =
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg1BQjH71U37XLfWqe+\
    /xP8iUrMiHpmUtbj3UfDkhFIrShRANCAAQgqHcxxwTVx1IXimcRv5TQyYZh+ShDM6X\
    ZqJonoP1m52oN0aLID1hJSrfKJrnqdgmHmaT4eXNNf4C5+g1HZt+u";

/// Constructs an EcdsaKeyPair from the default ingestor server.
pub fn default_ingestor_private_key() -> EcdsaKeyPair {
    EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_FIXED_SIGNING,
        &default_ingestor_private_key_raw(),
    )
    .map_err(|e| Error::CryptographyError("failed to parse ingestor key pair".to_owned(), e))
    // Since we know DEFAULT_INGESTOR_PRIVATE_KEY is valid, it
    // is ok to unwrap() here.
    .unwrap()
}

pub fn default_ingestor_private_key_raw() -> Vec<u8> {
    base64::decode(DEFAULT_INGESTOR_PRIVATE_KEY).unwrap()
}

pub fn default_ingestor_public_key() -> UnparsedPublicKey<Vec<u8>> {
    UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_FIXED,
        default_ingestor_private_key()
            .public_key()
            .as_ref()
            .to_vec(),
    )
}

pub fn default_facilitator_signing_private_key() -> EcdsaKeyPair {
    EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_FIXED_SIGNING,
        &default_facilitator_signing_private_key_raw(),
    )
    .map_err(|e| Error::CryptographyError("failed to parse ingestor key pair".to_owned(), e))
    .unwrap()
}

pub fn default_facilitator_signing_private_key_raw() -> Vec<u8> {
    base64::decode(DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY).unwrap()
}

pub fn default_facilitator_signing_public_key() -> UnparsedPublicKey<Vec<u8>> {
    UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_FIXED,
        default_facilitator_signing_private_key()
            .public_key()
            .as_ref()
            .to_vec(),
    )
}

pub fn default_pha_signing_private_key() -> Vec<u8> {
    base64::decode(DEFAULT_PHA_SIGNING_PRIVATE_KEY).unwrap()
}
