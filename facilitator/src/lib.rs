use prio::encrypt::EncryptError;
use ring::digest;
use std::io::Write;
use std::num::TryFromIntError;

pub mod aggregation;
pub mod batch;
pub mod idl;
pub mod intake;
pub mod sample;
pub mod test_utils;
pub mod transport;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";

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
    CryptographyError(String, ring::error::KeyRejected),
    CryptographyUnspecifiedError(String, ring::error::Unspecified),
    PeerValidationError(String),
    ConversionError(String),
}

impl From<TryFromIntError> for Error {
    fn from(e: TryFromIntError) -> Self {
        Error::ConversionError(format!("failed to convert value: {}", e))
    }
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
