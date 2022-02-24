#![allow(clippy::too_many_arguments)]

use aggregation::AggregationError;
use anyhow::Result;
use intake::IntakeError;
use ring::{digest, signature::EcdsaKeyPair};
use std::io::Write;
use url::Url;

pub mod aggregation;
pub mod aws_credentials;
pub mod batch;
pub mod config;
pub mod gcp_oauth;
pub mod http;
pub mod idl;
pub mod intake;
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
    #[error("HTTP resource error: {0}")]
    HttpError(#[from] ureq::Error),
    #[error("error parsing time: {0}")]
    TimeParse(#[from] chrono::ParseError),
    #[error("command line parsing error: {0}")]
    Clap(#[from] clap::Error),
    #[error("missing arguments: {0}")]
    MissingArguments(&'static str),
    #[error(transparent)]
    Intake(#[from] IntakeError),
    #[error(transparent)]
    Aggregation(AggregationError),
    #[error(transparent)]
    Url(#[from] UrlParseError),
    #[error("failed to deserialize JSON key file: {0}")]
    BadKeyFile(serde_json::Error),
}

/// This trait captures whether a given error is due to corruption in client-provided data, in
/// which case it is unnecessary to retry its processing, or due to I/O errors or cloud service
/// API errors, in which case processing should be retried at a later time.
pub trait ErrorClassification {
    fn is_retryable(&self) -> bool;
}

impl ErrorClassification for Error {
    fn is_retryable(&self) -> bool {
        match self {
            // Catch-all error type -- retries OK.
            Error::AnyhowError(_) => true,
            // Errors from ureq are obviously retryable.
            Error::HttpError(_) => true,
            // These errors likely indicate a problem with how this process was invoked, its
            // environment, or subsequent parsing of data from an outside source. As such,
            // the batch itself should be retried.
            Error::Clap(_)
            | Error::MissingArguments(_)
            | Error::TimeParse(_)
            | Error::BadKeyFile(_)
            | Error::Url(_) => true,
            // Dispatch to the wrapped error type.
            Error::Intake(e) => e.is_retryable(),
            Error::Aggregation(e) => e.is_retryable(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("failed to parse: {1}, {0}")]
pub struct UrlParseError(url::ParseError, String);

pub fn parse_url(input: String) -> Result<Url, UrlParseError> {
    Url::parse(&input).map_err(|e| UrlParseError(e, input))
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
