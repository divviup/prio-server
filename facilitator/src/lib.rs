use anyhow::Result;
use ring::{digest, signature::EcdsaKeyPair};
use std::io::Write;

pub mod aggregation;
pub mod aws_credentials;
pub mod batch;
pub mod config;
mod gcp_oauth;
pub mod http;
pub mod idl;
pub mod intake;
pub mod kubernetes;
pub mod logging;
pub mod manifest;
pub mod metrics;
mod retries;
pub mod sample;
pub mod task;
pub mod test_utils;
pub mod transport;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";

#[allow(clippy::large_enum_variant)]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    AnyhowError(#[from] anyhow::Error),
    #[error("avro error: {0}")]
    AvroError(String, #[source] avro_rs::Error),
    #[error("malformed header: {0}")]
    MalformedHeaderError(String),
    #[error("malformed data packet: {0}")]
    MalformedDataPacketError(String),
    #[error("end of file")]
    EofError,
    #[error("HTTP resource error")]
    HttpError(#[from] ureq::Error),
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
