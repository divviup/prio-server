mod gcs;
mod local;
mod s3;

use crate::{manifest::BatchSigningPublicKeys, BatchSigningKey};
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
use self::{gcs::GcsError, s3::S3Error};
pub use gcs::GcsTransport;
pub use local::LocalFileTransport;

/// Common error type for I/O over any batch transport.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("object {0} not found: {1}")]
    ObjectNotFoundError(String, anyhow::Error),
    #[error(transparent)]
    Local(#[from] local::FileError),
    #[error(transparent)]
    Gcs(#[from] GcsError),
    #[error(transparent)]
    S3(#[from] S3Error),
    #[error("an error occurred while cancelling after another error: {original}, {cancellation}")]
    Cancellation {
        original: Box<TransportError>,
        cancellation: Box<TransportError>,
    },
}

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
    /// canceled when the TransportWriter value is dropped. It is an error to
    /// call this without first having written some content.
    ///
    /// The `TransportWriter` cannot be used after this method is called,
    /// regardlesss of whether it succeeds.
    fn complete_upload(&mut self) -> Result<(), TransportError>;

    /// Cancel an upload operation, cleaning up any related resources. This
    /// method will be called automatically when the TransportWriter is dropped
    /// if complete_upload was not called successfully; errors will be logged.
    /// Callers may call this method manually in order to handle the case that
    /// an error occurs.
    ///
    /// The `TransportWriter` cannot be used after this method is called,
    /// regardlesss of whether it succeeds.
    fn cancel_upload(&mut self) -> Result<(), TransportError>;
}

impl<T: TransportWriter + ?Sized> TransportWriter for Box<T> {
    fn complete_upload(&mut self) -> Result<(), TransportError> {
        (**self).complete_upload()
    }

    fn cancel_upload(&mut self) -> Result<(), TransportError> {
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
    fn get(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn Read>, TransportError>;

    /// Returns an std::io::Write instance into which the contents of the value
    /// may be written.
    fn put(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn TransportWriter>, TransportError>;

    fn path(&self) -> String;
}

clone_trait_object!(Transport);
