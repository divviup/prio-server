use libprio_rs::encrypt::EncryptError;
use ring::{
    digest,
    signature::{
        EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED,
        ECDSA_P256_SHA256_FIXED_SIGNING,
    },
};
use std::io::Write;
use std::num::TryFromIntError;

pub mod aggregation;
pub mod batch;
pub mod idl;
pub mod intake;
pub mod sample;
pub mod transport;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";

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
    PeerValidationError(String),
    ConversionError(String),
}

impl From<TryFromIntError> for Error {
    fn from(e: TryFromIntError) -> Self {
        Error::ConversionError(format!("failed to convert value: {}", e))
    }
}

/// Constructs an EcdsaKeyPair from the default ingestor server. Should only be
/// used in tests, and since we know DEFAULT_INGESTOR_PRIVATE_KEY is valid, it
/// is ok to unwrap() here.
pub fn default_ingestor_private_key() -> EcdsaKeyPair {
    EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_FIXED_SIGNING,
        &default_ingestor_private_key_raw(),
    )
    .map_err(|e| {
        Error::CryptographyError(
            "failed to parse ingestor key pair".to_owned(),
            Some(e),
            None,
        )
    })
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
    .map_err(|e| {
        Error::CryptographyError(
            "failed to parse ingestor key pair".to_owned(),
            Some(e),
            None,
        )
    })
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

pub struct DigestWriter {
    context: digest::Context,
}

impl DigestWriter {
    fn new() -> DigestWriter {
        DigestWriter {
            context: digest::Context::new(&digest::SHA256),
        }
    }

    fn finish(self) -> digest::Digest {
        self.context.finish()
    }
}

impl Write for DigestWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        self.context.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

/// MemoryWriter is a trait we apply to std::io::Write implementations that are
/// backed by memory and are assumed to not fail or perform short writes in any
/// circumstance short of ENOMEM.
pub trait MemoryWriter: Write {}

impl MemoryWriter for DigestWriter {}
impl MemoryWriter for Vec<u8> {}

/// SidecarWriter wraps an std::io::Write, but also writes any buffers passed to
/// it into a MemoryWriter. The reason sidecar isn't simply some other type that
/// implements std::io::Write is that SidecarWriter::write assumes that short
/// writes to the sidecar can't happen, so we use the trait to ensure that this
/// property holds.
pub struct SidecarWriter<W: Write, M: MemoryWriter> {
    writer: W,
    sidecar: M,
}

impl<W: Write, M: MemoryWriter> SidecarWriter<W, M> {
    fn new(writer: W, sidecar: M) -> SidecarWriter<W, M> {
        SidecarWriter {
            writer: writer,
            sidecar: sidecar,
        }
    }
}

impl<W: Write, M: MemoryWriter> Write for SidecarWriter<W, M> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        // We might get a short write into whatever writer is, so make sure the
        // sidecar writer doesn't get ahead in that case.
        let n = self.writer.write(buf)?;
        self.sidecar.write_all(&buf[..n])?;
        Ok(n)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.writer.flush()
    }
}
