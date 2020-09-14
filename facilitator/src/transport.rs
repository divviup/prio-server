use crate::Error;
use std::boxed::Box;
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::{Cursor, Read, Write};
use std::ops::{Deref, DerefMut};
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
pub struct FileTransport {
    directory: PathBuf,
}

impl FileTransport {
    /// Creates a FileTransport under the specified path. The key parameter
    /// provided to `put` or `get` will be interpreted as a relative path.
    pub fn new(directory: PathBuf) -> FileTransport {
        FileTransport {
            directory: directory,
        }
    }
}

impl FileTransport {
    pub fn get_impl(&self, key: &Path) -> Result<File, Error> {
        let path = self.directory.join(key);
        let f = File::open(path.as_path())
            .map_err(|e| Error::IoError(format!("opening {}", path.display()), e))?;
        Ok(f)
    }

    pub fn put_impl(&mut self, key: &Path) -> Result<File, Error> {
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
        Ok(f)
    }
}

impl Transport for FileTransport {
    fn get(&self, key: &Path) -> Result<Box<dyn Read>, Error> {
        self.get_impl(key).map(|v| Box::new(v) as Box<dyn Read>)
    }

    fn put(&mut self, key: &Path) -> Result<Box<dyn Write>, Error> {
        self.put_impl(key).map(|v| Box::new(v) as Box<dyn Write>)
    }
}

/// An adapter provided an std::io::Write implementation for a RefMut<Vec<u8>>.
/// RefMut<Vec<u8>> can automatically be dereferenced into Vec<u8>, but that
/// type only implements std::io::Write, not std::io::Read.
pub struct WritableVecRefMut<'a> {
    inner: RefMut<'a, Vec<u8>>,
}

impl WritableVecRefMut<'_> {
    fn new(ref_cell: &RefCell<Vec<u8>>) -> WritableVecRefMut {
        WritableVecRefMut {
            inner: ref_cell.borrow_mut(),
        }
    }
}

impl Write for WritableVecRefMut<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.deref_mut().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.deref_mut().flush()
    }
}

/// A Transport implementation backed by a HashMap of Vec<u8>.
pub struct MemoryTransport {
    pub contents: HashMap<PathBuf, RefCell<Vec<u8>>>,
}

impl MemoryTransport {
    pub fn new() -> MemoryTransport {
        MemoryTransport {
            contents: HashMap::new(),
        }
    }

    pub fn get_impl(&self, key: &Path) -> Result<Cursor<Vec<u8>>, Error> {
        match self.contents.get(&key.to_path_buf()) {
            Some(rc) => Ok(Cursor::new(rc.borrow().deref().to_vec())),
            None => Err(Error::IoError(
                format!("no value for key {}", key.display()),
                std::io::Error::new(std::io::ErrorKind::NotFound, ""),
            )),
        }
    }

    pub fn put_impl(&mut self, key: &Path) -> Result<WritableVecRefMut, Error> {
        if !self.contents.contains_key(&key.to_path_buf()) {
            self.contents
                .insert(key.to_path_buf(), RefCell::new(Vec::new()));
        }
        // We just guaranteed that the key is present, making unwrap() safe.
        Ok(WritableVecRefMut::new(
            self.contents.get(&key.to_path_buf()).unwrap(),
        ))
    }
}

impl Transport for MemoryTransport {
    fn get(&self, key: &Path) -> Result<Box<dyn Read>, Error> {
        self.get_impl(key).map(|v| Box::new(v) as Box<dyn Read>)
    }

    fn put(&mut self, _key: &Path) -> Result<Box<dyn Write>, Error> {
        // TODO(timg) the line below doesn't compile and I can't figure out why
        // -- something to do with the lifetime of the value returned from
        // put_impl when it goes into the Box.
        //self.put_impl(_key).map(|v| Box::new(v) as Box<dyn Write>)
        panic!("not implemented")
    }
}

/// Trivial implementor of std::io::{Read, Write} used in NullTransport.
struct NullReadWriter {}

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
        let mut file_transport = FileTransport::new(tempdir.path().to_path_buf());
        let content = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let path = Path::new("path");
        let path2 = Path::new("path2");
        let complex_path = Path::new("path3/with/separators");

        {
            let ret = file_transport.get_impl(&path2);
            assert!(ret.is_err(), "unexpected return value {:?}", ret.err());
        }

        for path in &[path, complex_path] {
            let writer = file_transport.put_impl(&path);
            assert!(writer.is_ok(), "unexpected error {:?}", writer.err());

            let res = writer.unwrap().write_all(&content);
            assert!(res.is_ok(), "failed to write {:?}", res.err());

            let reader = file_transport.get_impl(&path);
            assert!(reader.is_ok(), "create reader failed: {:?}", reader.err());

            let mut content_again = Vec::new();
            let res = reader.unwrap().read_to_end(&mut content_again);
            assert!(res.is_ok(), "failed to read: {:?}", res.err());
            assert_eq!(content_again, content);
        }
    }

    #[test]
    fn roundtrip_file_transport_trait() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let file_transport = FileTransport::new(tempdir.path().to_path_buf());
        do_roundtrip_transport_trait(file_transport);
    }

    fn do_roundtrip_transport_trait(mut file_transport: impl Transport) {
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

            let res = writer.unwrap().write_all(&content);
            assert!(res.is_ok(), "failed to write {:?}", res.err());

            let reader = file_transport.get(&path);
            assert!(reader.is_ok(), "create reader failed: {:?}", reader.err());

            let mut content_again = Vec::new();
            let res = reader.unwrap().read_to_end(&mut content_again);
            assert!(res.is_ok(), "failed to read: {:?}", res.err());
            assert_eq!(content_again, content);
        }
    }

    #[test]
    #[ignore]
    fn roundtrip_memory_transport_trait() {
        // ignore this test until we can figure out the lifetime issue
        let memory_transport = MemoryTransport::new();
        do_roundtrip_transport_trait(memory_transport);
    }

    #[test]
    fn roundtrip_memory_transport() {
        let mut memory_transport = MemoryTransport::new();
        let content = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let path = Path::new("path");
        let path2 = Path::new("path2");
        let complex_path = Path::new("path/with/separators");

        {
            let ret = memory_transport.get_impl(&path2);
            assert!(ret.is_err(), "unexpected return value {:?}", ret);
        }

        for path in &[path, complex_path] {
            let writer = memory_transport.put_impl(&path);
            assert!(writer.is_ok(), "unexpected error {:?}", writer.err());

            let res = writer.unwrap().write_all(&content);
            assert!(res.is_ok(), "failed to write {:?}", res.err());

            let reader = memory_transport.get_impl(&path);
            assert!(reader.is_ok(), "create reader failed: {:?}", reader.err());

            let mut content_again = Vec::new();
            let res = reader.unwrap().read_to_end(&mut content_again);
            assert!(res.is_ok(), "failed to read: {:?}", res.err());
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

        let res = writer.unwrap().write_all(&content);
        assert!(res.is_ok(), "failed to write {:?}", res.err());

        let reader = null_transport.get(&path);
        assert!(reader.is_ok(), "create reader failed: {:?}", reader.err());

        let mut content_again = Vec::new();
        let res = reader.unwrap().read_to_end(&mut content_again);
        assert!(res.is_ok(), "failed to read: {:?}", res.err());
        assert_eq!(
            content_again.len(),
            0,
            "vector unexpectedly contents: {:?}",
            content_again
        );
    }
}
