use crate::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

pub trait Transport {
    fn get(&self, key: &str) -> Result<Box<dyn Read>, Error>;
    fn put(&self, key: &str) -> Result<Box<dyn Write>, Error>;
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
    fn get(&self, key: &str) -> Result<Box<dyn Read>, Error> {
        let path = self.directory.join(key);
        let f = File::open(path.as_path()).map_err(|e| Error::IoError(e))?;
        Ok(Box::new(BufReader::new(f)))
    }

    fn put(&self, key: &str) -> Result<Box<dyn Write>, Error> {
        let path = self.directory.join(key);
        let f = File::create(path.as_path()).map_err(|e| Error::IoError(e))?;
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
        let writer = file_transport.put("foo");
        assert!(writer.is_ok(), "put failed: {:?}", writer.err());

        let res = writer.unwrap().write(b"some content");
        assert!(res.is_ok(), "write failed: {:?}", res.err());

        let reader = file_transport.get("foo");
        assert!(reader.is_ok(), "get failed: {:?}", reader.err());

        let mut contents = String::new();
        let res = reader.unwrap().read_to_string(&mut contents);
        assert!(res.is_ok(), "read failed: {:?}", res.err());
        assert_eq!(contents, "some content");
    }
}
