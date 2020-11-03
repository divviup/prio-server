use anyhow::Result;
use ring::{digest, signature::EcdsaKeyPair};
use std::io::Write;

pub mod aggregation;
pub mod batch;
pub mod config;
pub mod idl;
pub mod intake;
pub mod manifest;
pub mod sample;
pub mod test_utils;
pub mod transport;
mod workflow;

pub use workflow::{workflow_main, WorkflowArgs};

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";

/// Identity represents a cloud identity: Either an AWS IAM ARN (i.e. "arn:...")
/// or a GCP ServiceAccount (i.e. "foo@bar.com").
pub type Identity = String;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Compatibility enum entry to ease migration.
    #[error(transparent)]
    // TODO(yuriks): The `From` impl should stay disabled while not updating errors, to avoid
    //   accidentally wrapping an `EofError` inside `AnyhowError`, which would then get missed when
    //   code tries to pattern-match against it. I don't trust our current test coverage enough to
    //   detect all cases where that happens and leads to incorrect behavior.
    //   Later on, `anyhow` downcasting with `Error::is` can be used to check for `EofError`.
    AnyhowError(/*#[from]*/ anyhow::Error),

    #[error("avro error: {0}")]
    AvroError(String, #[source] avro_rs::Error),
    #[error("malformed header: {0}")]
    MalformedHeaderError(String),
    #[error("malformed data packet: {0}")]
    MalformedDataPacketError(String),
    #[error("end of file")]
    EofError,
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
pub struct BatchSigningKey {
    /// The ECDSA P256 key pair to use when signing batches.
    pub key: EcdsaKeyPair,
    /// The key identifier to be inserted into signature structures, which
    /// must correspond to a batch-signing-key in the data share processor's
    /// specific manifest.
    pub identifier: String,
}

#[cfg(test)]
mod tests {
    use crate::DigestWriter;
    use std::io::{Write};

    #[test]
    fn digest_writer_test() {
        let mut writer = DigestWriter::new();
        let written = writer.write("I expect to be written into sha256".to_string().as_bytes()).unwrap();

        assert_eq!(written, 34);

        let digest = writer.finish();
        let sha = digest.as_ref();

        let hexed_sha = format!("{:02x?}", sha);
        let hexed_sha = hexed_sha.replace(|ch| !char::is_alphanumeric(ch), "");

        assert_eq!(hexed_sha, "b1b64ca32c118bfd5d1f40fdb25314468f82c0e9427f4f107ddfa89ce357a3ec".to_string())
    }
}

