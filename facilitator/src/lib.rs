use anyhow::Result;
use ring::{digest, signature::EcdsaKeyPair};
use std::io::Write;

pub mod aggregation;
pub mod aws_credentials;
pub mod batch;
pub mod config;
pub mod gcp_oauth;
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
    #[error("malformed batch: {0}")]
    MalformedBatchError(String),
    #[error("end of file")]
    EofError,
    #[error("HTTP resource error")]
    HttpError(#[from] ureq::Error),
}

/// A wrapper-writer that computes a SHA256 digest over the content it is provided.
pub struct DigestWriter<W: Write> {
    writer: W,
    context: digest::Context,
}

impl<W: Write> DigestWriter<W> {
    fn new(writer: W) -> DigestWriter<W> {
        DigestWriter {
            writer,
            context: digest::Context::new(&digest::SHA256),
        }
    }

    /// Consumes the DigestWriter and returns the computed SHA256 hash.
    fn finish(self) -> digest::Digest {
        self.context.finish()
    }
}

impl<W: Write> Write for DigestWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        let result = self.writer.write(buf);
        if let Ok(n) = result {
            self.context.update(&buf[..n]);
        }
        result
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.writer.flush()
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

#[cfg(test)]
mod tests {
    use crate::DigestWriter;
    use std::io::Write;

    #[test]
    fn digest_writer_test() {
        const TEST_STR: &[u8] = b"I expect to be written into sha256";
        const TEST_STR_DIGEST: &str =
            "b1b64ca32c118bfd5d1f40fdb25314468f82c0e9427f4f107ddfa89ce357a3ec"; // verified via sha256sum

        let mut written: Vec<u8> = Vec::new();
        let mut writer = DigestWriter::new(&mut written);
        let written_cnt = writer.write(TEST_STR).unwrap();
        let digest = hex::encode(writer.finish().as_ref());

        assert_eq!(written_cnt, TEST_STR.len());
        assert_eq!(&written[..], TEST_STR);
        assert_eq!(&digest, TEST_STR_DIGEST);
    }
}
