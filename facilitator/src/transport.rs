mod gcs;
mod local;
mod s3;

use crate::{manifest::BatchSigningPublicKeys, BatchSigningKey};
use anyhow::Result;
use derivative::Derivative;
use dyn_clone::{clone_trait_object, DynClone};
use prio::encrypt::PrivateKey;
use std::{
    boxed::Box,
    fmt::Debug,
    io::{Read, Write},
};
use uuid::Uuid;

pub use self::s3::S3Transport;
pub use gcs::GcsTransport;
pub use local::LocalFileTransport;

/// A transport along with the public keys that can be used to verify signatures
/// on the batches read from the transport.
#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub struct VerifiableTransport {
    pub transport: Box<dyn Transport>,
    #[derivative(Debug = "ignore")]
    pub batch_signing_public_keys: BatchSigningPublicKeys,
}

#[derive(Clone, Debug)]
pub struct VerifiableAndDecryptableTransport {
    pub transport: VerifiableTransport,
    pub packet_decryption_keys: Vec<PrivateKey>,
}

#[derive(Debug)]
pub struct SignableTransport {
    pub transport: Box<dyn Transport>,
    pub batch_signing_key: BatchSigningKey,
}

/// A TransportWriter extends std::io::Write but adds methods that explicitly
/// allow callers to complete or cancel an upload.
pub trait TransportWriter: Write {
    /// Complete an upload operation, flushing any buffered writes and cleaning
    /// up any related resources. Callers must call this method to successfully
    /// finish an upload. If this method is not called, the upload will be
    /// canceled when the TransportWriter value is dropped.
    fn complete_upload(&mut self) -> Result<()>;

    /// Cancel an upload operation, cleaning up any related resources. This
    /// method will be called automatically when the TransportWriter is dropped
    /// if complete_upload was not called successfully; errors will be logged.
    /// Callers may call this method manually in order to handle the case that
    /// an error occurs.
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
/// object store like Amazon S3, or local files, or buffers in memory. The get()
/// and put() methods take a trace_id parameter which should be the unique trace
/// ID of the task that the get or put operation is a part of. These are
/// provided as parameters to these methods rather than being fields on the
/// Transport's own Logger because a single Transport could be re-used across
/// many tasks.
pub trait Transport: Debug + DynClone + Send {
    /// Returns an std::io::Read instance from which the contents of the value
    /// of the provided key may be read. If no object is found, an error
    /// wrapping ObjectNotFoundError is returned.
    fn get(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn Read>>;

    /// Returns an std::io::Write instance into which the contents of the value
    /// may be written.
    fn put(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn TransportWriter>>;

    fn path(&self) -> String;
}

clone_trait_object!(Transport);
