use crate::Error;
use std::boxed::Box;
use std::fs::{create_dir_all, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// A transport moves object in and out of some data store, such as a cloud
/// object store like Amazon S3, or local files, or buffers in memory.
pub trait Transport {
    /// Returns an std::io::Read instance from which the contents of the value
    /// of the provided key may be read.
    fn get(&self, key: &Path) -> Result<Box<dyn Read>, Error>;
    /// Returns an std::io::Write instance into which the contents of the value
    /// may be written.
    fn put(&mut self, key: &Path) -> Result<Box<dyn Write>, Error>;
}

/// A transport implementation backed by the local filesystem.
pub struct LocalFileTransport {
    directory: PathBuf,
}

impl LocalFileTransport {
    /// Creates a LocalFileTransport under the specified path. The key parameter
    /// provided to `put` or `get` will be interpreted as a relative path.
    pub fn new(directory: PathBuf) -> LocalFileTransport {
        LocalFileTransport {
            directory: directory,
        }
    }
}

impl Transport for LocalFileTransport {
    fn get(&self, key: &Path) -> Result<Box<dyn Read>, Error> {
        let path = self.directory.join(key);
        let f = File::open(path.as_path())
            .map_err(|e| Error::IoError(format!("opening {}", path.display()), e))?;
        Ok(Box::new(f))
    }

    fn put(&mut self, key: &Path) -> Result<Box<dyn Write>, Error> {
        let path = self.directory.join(key);
        if let Some(parent) = path.parent() {
            create_dir_all(parent).map_err(|e| {
                Error::IoError(
                    format!("creating parent directories {}", parent.display()),
                    e,
                )
            })?;
        }
        let f = File::create(path.as_path())
            .map_err(|e| Error::IoError(format!("creating {}", path.display()), e))?;
        Ok(Box::new(f))
    }
}

/// Trivial implementor of std::io::{Read, Write} used in NullTransport.
struct NullReadWriter;

impl Read for NullReadWriter {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Ok(0)
    }
}

impl Write for NullReadWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Implementation of Transport that simply discards all writes and whose reads
/// always yield 0 bytes. Intended for use in tests, e.g. when a client is set
/// up but only needs to emit shares to one server.
pub struct NullTransport {}

impl Transport for NullTransport {
    fn get(&self, _key: &Path) -> Result<Box<dyn Read>, Error> {
        Ok(Box::new(NullReadWriter {}))
    }

    fn put(&mut self, _key: &Path) -> Result<Box<dyn Write>, Error> {
        Ok(Box::new(NullReadWriter {}))
    }
}

// TODO: S3Transport; https://github.com/rusoto/rusoto

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn roundtrip_file_transport() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let mut file_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let content = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let path = Path::new("path");
        let path2 = Path::new("path2");
        let complex_path = Path::new("path3/with/separators");

        {
            let ret = file_transport.get(&path2);
            assert!(ret.is_err(), "unexpected return value {:?}", ret.err());
        }

        for path in &[path, complex_path] {
            let writer = file_transport.put(&path);
            assert!(writer.is_ok(), "unexpected error {:?}", writer.err());

            writer
                .unwrap()
                .write_all(&content)
                .expect("failed to write");

            let reader = file_transport.get(&path);
            assert!(reader.is_ok(), "create reader failed: {:?}", reader.err());

            let mut content_again = Vec::new();
            reader
                .unwrap()
                .read_to_end(&mut content_again)
                .expect("failed to read");
            assert_eq!(content_again, content);
        }
    }

    #[test]
    fn roundtrip_null_transport() {
        let mut null_transport = NullTransport {};
        let content = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let path = Path::new("path");

        let writer = null_transport.put(&path);
        assert!(writer.is_ok(), "unexpected error {:?}", writer.err());

        writer
            .unwrap()
            .write_all(&content)
            .expect("failed to write");

        let reader = null_transport.get(&path);
        assert!(reader.is_ok(), "create reader failed: {:?}", reader.err());

        let mut content_again = Vec::new();
        reader
            .unwrap()
            .read_to_end(&mut content_again)
            .expect("failed to read");
        assert_eq!(
            content_again.len(),
            0,
            "vector unexpectedly contents: {:?}",
            content_again
        );
    }
}
