use anyhow::Result;
use ring::{digest, signature::EcdsaKeyPair};
use std::io::Write;

pub mod aggregation;
mod aws_credentials;
pub mod batch;
pub mod config;
mod gcp_oauth;
pub mod http;
pub mod idl;
pub mod intake;
pub mod manifest;
pub mod metrics;
pub mod sample;
pub mod task;
pub mod test_utils;
pub mod transport;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An error occurred while reading or writing an ingestion batch
    #[error("error reading/writing ingestion batch")]
    BadIngestionBatch(#[source] Box<Error>),
    /// An error occurred while reading or writing a peer validation batch
    #[error("error reading/writing peer validation batch")]
    BadPeerValidationBatch(#[source] Box<Error>),
    /// An error occurred while reading or writing an own validation batch
    #[error("error reading/writing own validation batch")]
    BadOwnValidationBatch(#[source] Box<Error>),

    // Errors from batch module
    #[error("{header_path}: key {key_name} not present in key map")]
    NoSuchKey {
        header_path: String,
        key_name: String,
    },
    #[error("invalid signature on header {header_path} with key {key_name}")]
    InvalidSignature {
        header_path: String,
        key_name: String,
    },
    #[error("packet file digest in header {header_path} {expected_digest} does not match actual packet file digest {actual_digest}")]
    DigestMismatch {
        header_path: String,
        expected_digest: String,
        actual_digest: String,
    },

    // Errors from aggregation module
    #[error("values in batch headers do not match: {lhs:?}\n{rhs:?}")]
    HeaderMismatch { lhs: String, rhs: String },

    // Errors from idl module
    #[error("avro error: {0}")]
    Avro(String, #[source] avro_rs::Error),
    #[error("malformed header: {0}")]
    MalformedHeader(String),
    #[error("malformed data packet: {0}")]
    MalformedDataPacket(String),
    #[error("end of file")]
    Eof,

    #[error("invalid value for argument {0}")]
    InvalidArgument(String),

    /// Catchall case for errors not captured by specific variants of this enum.
    /// As we encounter more errors that should be distinguished from others
    /// programmatically, specific variants should be added to this enum.
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

/// An implementation of transport::TransportWriter that computes a SHA256
/// digest over the content it is provided.
pub struct DigestWriter {
    context: digest::Context,
}

impl DigestWriter {
    fn new() -> DigestWriter {
        DigestWriter {
            context: digest::Context::new(&digest::SHA256),
        }
    }

    /// Consumes the DigestWriter and returns the computed SHA256 hash.
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

/// SidecarWriter wraps a vector of std::io::Writes of one type and writes all
/// provided buffers to them. It also writes all buffers to an additional
/// instance of std::io:Write that may be of a different type than the ones in
/// the vector.
pub struct SidecarWriter<T: Write, W: Write> {
    writers: Vec<T>,
    sidecar: W,
}

impl<T: Write, W: Write> SidecarWriter<T, W> {
    fn new(writers: Vec<T>, sidecar: W) -> SidecarWriter<T, W> {
        SidecarWriter { writers, sidecar }
    }
}

impl<T: Write, W: Write> Write for SidecarWriter<T, W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        for writer in &mut self.writers {
            writer.write_all(buf)?;
        }
        self.sidecar.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        for writer in &mut self.writers {
            writer.flush()?;
        }
        self.sidecar.flush()
    }
}

/// This struct represents a key used by this data share processor to sign
/// batches (ingestion, validation or sum part).
#[derive(Debug)]
pub struct BatchSigningKey {
    /// The ECDSA P256 key pair to use when signing batches.
    pub key: EcdsaKeyPair,
    /// The key identifier to be inserted into signature structures, which
    /// must correspond to a batch-signing-key in the data share processor's
    /// specific manifest.
    pub identifier: String,
}

/// Pretty print a byte array as a hex string.
fn hex_dump(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .concat()
}

#[cfg(test)]
mod tests {
    use crate::DigestWriter;
    use std::io::Write;

    #[test]
    fn digest_writer_test() {
        let mut writer = DigestWriter::new();
        let written = writer
            .write("I expect to be written into sha256".to_string().as_bytes())
            .unwrap();

        assert_eq!(written, 34);

        let digest = writer.finish();
        let sha = digest.as_ref();

        let hexed_sha = format!("{:02x?}", sha);
        let hexed_sha = hexed_sha.replace(|ch| !char::is_alphanumeric(ch), "");

        assert_eq!(
            hexed_sha,
            "b1b64ca32c118bfd5d1f40fdb25314468f82c0e9427f4f107ddfa89ce357a3ec".to_string()
        )
    }
}
