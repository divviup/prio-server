use anyhow::Result;
use ring::digest;
use std::io::Write;

pub mod aggregation;
pub mod batch;
pub mod idl;
pub mod intake;
pub mod manifest;
pub mod sample;
pub mod test_utils;
pub mod transport;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";

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

/// SidecarWriter wraps an std::io::Write, but also writes any buffers passed to
/// it into a second std::io::Write.
pub struct SidecarWriter<T: Write, W: Write> {
    writer: T,
    sidecar: W,
}

impl<T: Write, W: Write> SidecarWriter<T, W> {
    fn new(writer: T, sidecar: W) -> SidecarWriter<T, W> {
        SidecarWriter { writer, sidecar }
    }
}

impl<T: Write, W: Write> Write for SidecarWriter<T, W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        // We might get a short write into whatever writer is, so make sure the
        // sidecar writer doesn't get ahead in that case.
        let n = self.writer.write(buf)?;
        self.sidecar.write_all(&buf[..n])?;
        Ok(n)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.writer.flush()?;
        self.sidecar.flush()
    }
}
