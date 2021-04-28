use crate::transport::{Transport, TransportWriter};
use anyhow::{Context, Result};

use std::{
    boxed::Box,
    fs::{create_dir_all, File},
    io::Read,
    path::{PathBuf, MAIN_SEPARATOR},
};

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
        PathBuf::from(key.replace("/", &MAIN_SEPARATOR.to_string()))
    }
}

impl Transport for LocalFileTransport {
    fn path(&self) -> String {
        self.directory.to_string_lossy().to_string()
    }

    fn get(&mut self, key: &str, _trace_id: &str) -> Result<Box<dyn Read>> {
        let path = self.directory.join(LocalFileTransport::relative_path(key));
        let f =
            File::open(path.as_path()).with_context(|| format!("opening {}", path.display()))?;
        Ok(Box::new(f))
    }

    fn put(&mut self, key: &str, _trace_id: &str) -> Result<Box<dyn TransportWriter>> {
        let path = self.directory.join(LocalFileTransport::relative_path(key));
        if let Some(parent) = path.parent() {
            create_dir_all(parent)
                .with_context(|| format!("creating parent directories {}", parent.display()))?;
        }
        let f =
            File::create(path.as_path()).with_context(|| format!("creating {}", path.display()))?;
        Ok(Box::new(f))
    }
}

impl TransportWriter for File {
    fn complete_upload(&mut self) -> Result<()> {
        // This method is a no-op for local files
        Ok(())
    }

    fn cancel_upload(&mut self) -> Result<()> {
        // This method is a no-op for local files
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_file_transport() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let mut file_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let content = vec![1, 2, 3, 4, 5, 6, 7, 8];

        {
            let ret = file_transport.get("path2", "");
            assert!(ret.is_err(), "unexpected return value {:?}", ret.err());
        }

        for path in &["path", "path3/with/separators"] {
            let writer = file_transport.put(path, "");
            assert!(writer.is_ok(), "unexpected error {:?}", writer.err());

            writer
                .unwrap()
                .write_all(&content)
                .expect("failed to write");

            let reader = file_transport.get(path, "");
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
