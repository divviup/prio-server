use crate::Error;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub trait Transport {
    fn get(&self, key: &Path) -> Result<Box<dyn Read>, Error>;
    fn put(&self, key: &Path) -> Result<Box<dyn Write>, Error>;
}

pub struct FileTransport {
    directory: PathBuf,
}

impl FileTransport {
    pub fn new(directory: PathBuf) -> FileTransport {
        FileTransport {
            directory: directory,
        }
    }
}

impl Transport for FileTransport {
    fn get(&self, key: &Path) -> Result<Box<dyn Read>, Error> {
        let path = self.directory.join(key);
        let f = File::open(path.as_path())
            .map_err(|e| Error::IoError(e, format!("opening {}", path.display())))?;
        Ok(Box::new(BufReader::new(f)))
    }

    fn put(&self, key: &Path) -> Result<Box<dyn Write>, Error> {
        let path = self.directory.join(key);
        if let Some(parent) = path.parent() {
            create_dir_all(parent).map_err(|e| {
                Error::IoError(
                    e,
                    format!("creating parent directories {}", parent.display()),
                )
            })?;
        }
        let f = File::create(path.as_path())
            .map_err(|e| Error::IoError(e, format!("creating {}", path.display())))?;
        Ok(Box::new(BufWriter::new(f)))
    }
}

// TODO: S3Transport; https://github.com/rusoto/rusoto

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_file_transport() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let file_transport = FileTransport::new(tempdir.path().to_path_buf());
        let writer = file_transport.put(Path::new("foo"));
        assert!(writer.is_ok(), "put failed: {:?}", writer.err());

        let res = writer.unwrap().write(b"some content");
        assert!(res.is_ok(), "write failed: {:?}", res.err());

        let reader = file_transport.get(Path::new("foo"));
        assert!(reader.is_ok(), "get failed: {:?}", reader.err());

        let mut contents = String::new();
        let res = reader.unwrap().read_to_string(&mut contents);
        assert!(res.is_ok(), "read failed: {:?}", res.err());
        assert_eq!(contents, "some content");
    }
}
