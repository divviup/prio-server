mod local;
mod s3;

use anyhow::Result;
use std::{
    boxed::Box,
    io::{Read, Write},
};

pub use local::LocalFileTransport;
pub use s3::S3Transport;

/// A TransportWriter extends std::io::Write but adds methods that explicitly
/// allow callers to complete or cancel an upload.
pub trait TransportWriter: Write {
    /// Complete an upload operation, flushing any buffered writes and cleaning
    /// up any related resources. Callers must call this method or cancel_upload
    /// when they are done with the TransportWriter.
    fn complete_upload(&mut self) -> Result<()>;

    /// Cancel an upload operation, cleaning up any related resources. Callers
    /// must call this method or complete_upload when  they are done with the
    /// Transportwriter.
    fn cancel_upload(&mut self) -> Result<()>;
}

impl<T: TransportWriter + ?Sized> TransportWriter for Box<T> {
    fn complete_upload(&mut self) -> Result<()> {
        (**self).complete_upload()
    }

    fn cancel_upload(&mut self) -> Result<()> {
        (**self).cancel_upload()
    }
}

/// A transport moves object in and out of some data store, such as a cloud
/// object store like Amazon S3, or local files, or buffers in memory.
pub trait Transport {
    /// Returns an std::io::Read instance from which the contents of the value
    /// of the provided key may be read.
    fn get(&self, key: &str) -> Result<Box<dyn Read>>;
    /// Returns an std::io::Write instance into which the contents of the value
    /// may be written.
    fn put(&mut self, key: &str) -> Result<Box<dyn TransportWriter>>;
}
