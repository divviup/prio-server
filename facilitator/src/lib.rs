use crate::idl::{IngestionDataSharePacket, ValidationPacket};
use anyhow::{anyhow, Context, Result};
use prio::{
    field::Field32,
    server::{Server, ServerError},
};
use ring::{digest, signature::EcdsaKeyPair};
use std::convert::TryFrom;
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

pub fn generate_validation_packet(
    servers: &mut Vec<Server<Field32>>,
    packet: &IngestionDataSharePacket,
) -> Result<ValidationPacket> {
    let r_pit = Field32::from(
        u32::try_from(packet.r_pit)
            .with_context(|| format!("illegal r_pit value {}", packet.r_pit))?,
    );

    // TODO(timg): if this fails for a non-empty subset of the
    // ingestion packets, do we abort handling of the entire
    // batch (as implemented currently) or should we record it
    // as an invalid UUID and emit a validation batch for the
    // other packets?
    for server in servers.iter_mut() {
        let validation_message = match server
            .generate_verification_message(r_pit, &packet.encrypted_payload)
        {
            Ok(m) => m,
            Err(ServerError::Encrypt(_)) => {
                continue;
            }
            Err(e) => {
                return Err(anyhow::Error::new(e).context("error generating verification message"));
            }
        };

        return Ok(ValidationPacket {
            uuid: packet.uuid,
            f_r: u32::from(validation_message.f_r) as i64,
            g_r: u32::from(validation_message.g_r) as i64,
            h_r: u32::from(validation_message.h_r) as i64,
        });
    }

    return Err(anyhow!(
        "failed to construct validation message for packet {} because all decryption attempts failed (key mismatch?)",
        packet.uuid
    ));
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
        let rslt = self.writer.write(buf);
        if let Ok(n) = rslt {
            self.context.update(&buf[..n]);
        }
        rslt
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
