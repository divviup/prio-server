use crate::{manifest::PacketEncryptionCertificateSigningRequest, BatchSigningKey};
use prio::encrypt::{PrivateKey, PublicKey};
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING,
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

// We have selected PEM armored, ASN.1 encoded PKIX SubjectPublicKeyInfo
// structures as the means of exchanging public keys with peer servers. However,
// no Rust crate that we have found gives us an easy way to obtain a PKIX SPKI
// from the PKCS#8 document format that ring uses for private key serialization.
// This constant and the other _SUBJECT_PUBLIC_KEY_INFO constants were obtained
// by placing the corresponding _PRIVATE_KEY constants into a PEM block, and
// then `openssl ec -inform PEM -outform PEM -in /path/to/PEM/PKCS#8/document
// -pubout`.
pub const DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO: &str =
    "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAENpmX5uFAu9z5FrGM91Lm+lEyABUA\
    jxsaLj9TtT5Zp7TqXy2Hb8mQwrnpwM79aAn8k6KJTQKzY8KP/oWqzZ9X7A==";
pub const DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY: &str =
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeSa+S+tmLupnAEyFK\
    dVuKB99y09YEqW41+8pwP4cTkahRANCAASy7FHcLGnRudVHWga/j2k9nQ3lMvuGE01\
    Q7DEyjyCuuw9YmB3dHvYcRUnxVRI/nF5LvneGim0dC7F1fuRAPeXI";
pub const DEFAULT_FACILITATOR_SUBJECT_PUBLIC_KEY_INFO: &str =
    "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEsuxR3Cxp0bnVR1oGv49pPZ0N5TL7\
    hhNNUOwxMo8grrsPWJgd3R72HEVJ8VUSP5xeS753hoptHQuxdX7kQD3lyA==";
pub const DEFAULT_PHA_SIGNING_PRIVATE_KEY: &str =
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg1BQjH71U37XLfWqe+\
    /xP8iUrMiHpmUtbj3UfDkhFIrShRANCAAQgqHcxxwTVx1IXimcRv5TQyYZh+ShDM6X\
    ZqJonoP1m52oN0aLID1hJSrfKJrnqdgmHmaT4eXNNf4C5+g1HZt+u";
pub const DEFAULT_PHA_SUBJECT_PUBLIC_KEY_INFO: &str =
    "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEIKh3MccE1cdSF4pnEb+U0MmGYfko\
    QzOl2aiaJ6D9ZudqDdGiyA9YSUq3yia56nYJh5mk+HlzTX+AufoNR2bfrg==";

pub const DEFAULT_PACKET_ENCRYPTION_CSR: &str =
    "MIHuMIGVAgEAMDMxMTAvBgNVBAMTKHVzLWN0LnByb2QtdXMuY2VydGlmaWNhdGVzL\
    mlzcmctcHJpby5vcmcwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAATp7xFbJGwHeMl\
    AW8W0cQ57qCPIBT5NBr2jR8a+Z1/QzQJRtJvR2pqbaJhWKw7y9ogp/TmcsaX+oP74+\
    SSGwrEYoAAwCgYIKoZIzj0EAwIDSAAwRQIgLSekh4unn6fLv9O9K4Lr6VxGEpLSqFz\
    259+Lrk7lwOkCIQCOzNvxwSb+iVFxJkaxUnxGYp2J+/2OnDGsKpyWY/wdhg==";

pub const DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST: &str =
    "-----BEGIN CERTIFICATE REQUEST-----\n\
    MIHyMIGZAgEAMDcxNTAzBgNVBAMTLG5hcm5pYS5hbWlyLWZhY2lsLmNlcnRpZmlj\n\
    YXRlcy5pc3JnLXByaW8ub3JnMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEImPq\n\
    nZJKMV0RUt4liCn/pzj1YEs8G13wvHNyqQ8HVmvjc7fj29K4vq5wkEXRK6QGD6UO\n\
    AZ9LiBsRN9cniLwysaAAMAoGCCqGSM49BAMCA0gAMEUCIQCqjVbLAabHDELRztvB\n\
    ZyzYemEzJoMqTRObEjpryr5gsgIgYAx8mMlBkI/GVJqCHvyBzMwRaz1hoQHme56H\n\
    vjjDWyI=\n\
    -----END CERTIFICATE REQUEST-----\n";

pub const DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST_PRIVATE_KEY: &str =
    "BCJj6p2SSjFdEVLeJYgp/6c49WBLPBtd8LxzcqkPB1Zr43O349vSuL6ucJBF0SukB\
    g+lDgGfS4gbETfXJ4i8MrHwu3/ts6VHR1/U9EIkHEFnEDZQ30r3NVASbEeJjd0/Ug==";

pub fn default_packet_encryption_certificate_signing_request(
) -> PacketEncryptionCertificateSigningRequest {
    PacketEncryptionCertificateSigningRequest::new(String::from(
        DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST,
    ))
}

/// Constructs an EcdsaKeyPair from the default ingestor server.
pub fn default_ingestor_private_key() -> BatchSigningKey {
    BatchSigningKey {
        key: EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_ASN1_SIGNING,
            &base64::decode(DEFAULT_INGESTOR_PRIVATE_KEY).unwrap(),
        )
        // Since we know DEFAULT_INGESTOR_PRIVATE_KEY is valid, it
        // is ok to unwrap() here.
        .unwrap(),
        identifier: "default-ingestor-signing-key".to_owned(),
    }
}

pub fn default_ingestor_public_key() -> UnparsedPublicKey<Vec<u8>> {
    UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_ASN1,
        default_ingestor_private_key()
            .key
            .public_key()
            .as_ref()
            .to_vec(),
    )
}

pub fn default_facilitator_signing_private_key() -> BatchSigningKey {
    BatchSigningKey {
        key: EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_ASN1_SIGNING,
            &base64::decode(DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY).unwrap(),
        )
        .unwrap(),
        identifier: "default-facilitator-signing-key".to_owned(),
    }
}

pub fn default_facilitator_signing_public_key() -> UnparsedPublicKey<Vec<u8>> {
    UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_ASN1,
        default_facilitator_signing_private_key()
            .key
            .public_key()
            .as_ref()
            .to_vec(),
    )
}

pub fn default_pha_signing_private_key() -> BatchSigningKey {
    BatchSigningKey {
        key: EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_ASN1_SIGNING,
            &base64::decode(DEFAULT_PHA_SIGNING_PRIVATE_KEY).unwrap(),
        )
        .unwrap(),
        identifier: "default-pha-signing-key".to_owned(),
    }
}

pub fn default_pha_signing_public_key() -> UnparsedPublicKey<Vec<u8>> {
    UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_ASN1,
        default_pha_signing_private_key()
            .key
            .public_key()
            .as_ref()
            .to_vec(),
    )
}

pub fn default_pha_packet_encryption_public_key() -> PublicKey {
    PublicKey::from(&PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap())
}

pub fn default_facilitator_packet_encryption_public_key() -> PublicKey {
    PublicKey::from(&PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap())
}
