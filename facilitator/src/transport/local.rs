use crate::transport::{Transport, TransportWriter};
use anyhow::Result;
use uuid::Uuid;

use std::{
    boxed::Box,
    fs::{create_dir_all, File},
    io::{ErrorKind, Read},
    path::{PathBuf, MAIN_SEPARATOR_STR},
};

use super::TransportError;

/// Errors that can arise when using the local filesystem as a batch transport.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("opening {1}, {0}")]
    OpenReading(std::io::Error, String),
    #[error("creating parent directories {1}, {0}")]
    Mkdirp(std::io::Error, String),
    #[error("creating {1}, {0}")]
    CreateFile(std::io::Error, String),
}

/// A transport implementation backed by the local filesystem.
#[derive(Clone, Debug)]
pub struct LocalFileTransport {
    directory: PathBuf,
}

impl LocalFileTransport {
    /// Creates a LocalFileTransport under the specified path. The key parameter
    /// provided to `put` or `get` will be interpreted as a relative path.
    pub fn new(directory: PathBuf) -> LocalFileTransport {
        LocalFileTransport { directory }
    }

    /// Callers will construct keys using "/" as a separator. This function
    /// attempts to convert the provided key into a relative path valid for the
    /// current platform.
    fn relative_path(key: &str) -> PathBuf {
        PathBuf::from(key.replace('/', MAIN_SEPARATOR_STR))
    }
}

impl Transport for LocalFileTransport {
    fn path(&self) -> String {
        self.directory.to_string_lossy().to_string()
    }

    fn get(&self, key: &str, _trace_id: &Uuid) -> Result<Box<dyn Read>, TransportError> {
        let path = self.directory.join(LocalFileTransport::relative_path(key));
        let f = File::open(path.as_path()).map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                return TransportError::ObjectNotFoundError(
                    key.to_owned(),
                    anyhow::Error::new(err),
                );
            }
            TransportError::Local(FileError::OpenReading(err, path.display().to_string()))
        })?;
        Ok(Box::new(f))
    }

    fn put(&self, key: &str, _trace_id: &Uuid) -> Result<Box<dyn TransportWriter>, TransportError> {
        let path = self.directory.join(LocalFileTransport::relative_path(key));
        if let Some(parent) = path.parent() {
            create_dir_all(parent)
                .map_err(|e| FileError::Mkdirp(e, parent.display().to_string()))?;
        }
        let f = File::create(path.as_path())
            .map_err(|e| FileError::CreateFile(e, path.display().to_string()))?;
        Ok(Box::new(f))
    }
}

impl TransportWriter for File {
    fn complete_upload(&mut self) -> Result<(), TransportError> {
        // This method is a no-op for local files
        Ok(())
    }

    fn cancel_upload(&mut self) -> Result<(), TransportError> {
        // This method is a no-op for local files
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::DEFAULT_TRACE_ID;

    #[test]
    fn roundtrip_file_transport() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let file_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let content = vec![1, 2, 3, 4, 5, 6, 7, 8];

        {
            let ret = file_transport.get("path2", &DEFAULT_TRACE_ID);
            assert!(ret.is_err(), "unexpected return value {:?}", ret.err());
        }

        for path in &["path", "path3/with/separators"] {
            let writer = file_transport.put(path, &DEFAULT_TRACE_ID);
            assert!(writer.is_ok(), "unexpected error {:?}", writer.err());

            writer
                .unwrap()
                .write_all(&content)
                .expect("failed to write");

            let reader = file_transport.get(path, &DEFAULT_TRACE_ID);
            assert!(reader.is_ok(), "create reader failed: {:?}", reader.err());

            let mut content_again = Vec::new();
            reader
                .unwrap()
                .read_to_end(&mut content_again)
                .expect("failed to read");
            assert_eq!(content_again, content);
        }
    }
}
