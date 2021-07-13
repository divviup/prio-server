use anyhow::{anyhow, Context, Result};
use rusoto_core::{region::ParseRegionError, Region};
use serde::{de, Deserialize, Deserializer};
use slog::Logger;

use std::{
    convert::Infallible,
    fmt::{self, Display, Formatter},
    path::PathBuf,
    str::FromStr,
};

use crate::aws_credentials;

/// Identity represents a cloud identity: Either an AWS IAM ARN (i.e. "arn:...")
/// or a GCP ServiceAccount (i.e. "foo@bar.com"). It treats the empty string as
/// equivalent to None, allowing arguments to unconditionally be provided to
/// facilitator.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    inner: Option<String>,
}

// We provide FromStr for clap::value_t
impl FromStr for Identity {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s))
    }
}

impl From<&str> for Identity {
    fn from(s: &str) -> Self {
        let inner = match s {
            "" => None,
            s => Some(s.to_owned()),
        };

        Self { inner }
    }
}

impl Display for Identity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner.as_deref().unwrap_or("default identity"))
    }
}

impl<'de> Deserialize<'de> for Identity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from(s.as_str()))
    }
}

impl Identity {
    pub fn none() -> Self {
        Self { inner: None }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        self.inner.as_deref()
    }
}

/// Parameters necessary to configure federation from AWS IAM to GCP IAM using
/// GCP workload identity pool
/// https://cloud.google.com/iam/docs/access-resources-aws
pub struct WorkloadIdentityPoolParameters {
    /// Full identifier of the workload identity pool provider, like
    /// "//iam.googleapis.com/projects/12345678/locations/global/workloadIdentityPools/my-pool/providers/my-aws-provider"
    pub workload_identity_pool_provider: String,
    /// A credential provider that can get AWS credentials for an AWS IAM role
    /// or user that is permitted to impersonate a GCP service account via the
    /// workload_identity_pool_provider
    pub aws_credentials_provider: aws_credentials::Provider,
}

impl WorkloadIdentityPoolParameters {
    /// Creates Some(WorkloadIdentityPoolParameters) suitable for use with
    /// GcpAccessTokenProvider if workload_identity_pool_provider_id is set and
    /// is not the empty string, or None otherwise.
    pub fn new(
        workload_identity_pool_provider_id: Option<&str>,
        use_default_aws_credentials_provider: bool,
        logger: &Logger,
    ) -> Result<Option<Self>> {
        let parameters = match workload_identity_pool_provider_id {
            // We treat the empty string as equivalent to None, to allow the
            // gcp-workload-identity-pool-provider command line argument to be
            // unconditionally set.
            None | Some("") => None,
            Some(provider_id) => Some(Self {
                workload_identity_pool_provider: provider_id.to_owned(),
                aws_credentials_provider: aws_credentials::Provider::new(
                    // We create this provider with no identity, effectively
                    // requiring that the authentication to AWS use either
                    // ambient AWS credentials or an EKS cluster OIDC provider.
                    Identity::none(),
                    use_default_aws_credentials_provider,
                    "IAM federation",
                    logger,
                )?,
            }),
        };

        Ok(parameters)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3Path {
    pub region: Region,
    pub bucket: String,
    pub key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum S3PathParseError {
    #[error("Not an S3 path")]
    NoPath,
    #[error(
        "S3 path must be in the format `s3://{{region}}/{{bucket name}}/{{optional key prefix}}`"
    )]
    InvalidFormat,
    #[error(transparent)]
    InvalidRegion(#[from] ParseRegionError),
}

impl S3Path {
    /// Returns `self`, possibly adding '/' at the end of the key to ensure it can be combined with another path as a directory prefix.
    pub fn ensure_directory_prefix(mut self) -> Self {
        if !self.key.is_empty() && !self.key.ends_with('/') {
            self.key.push('/');
        }
        self
    }
}

impl Display for S3Path {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "s3://{}/{}/{}",
            self.region.name(),
            self.bucket,
            self.key
        )
    }
}

impl FromStr for S3Path {
    type Err = S3PathParseError;

    fn from_str(s: &str) -> Result<Self, S3PathParseError> {
        let region_and_bucket = s.strip_prefix("s3://").ok_or(S3PathParseError::NoPath)?;

        // All we require is that the string contain a region and a bucket name.
        // Further validation of bucket names is left to Amazon servers.
        let mut components = region_and_bucket
            .splitn(3, '/')
            .take_while(|s| !s.is_empty());
        let region = Region::from_str(components.next().ok_or(S3PathParseError::InvalidFormat)?)?;
        let bucket = components
            .next()
            .ok_or(S3PathParseError::InvalidFormat)?
            .to_owned();
        let key = components.next().map(|s| s.to_owned()).unwrap_or_default();
        // splitn will only return 3 so it should never have more
        assert!(components.next().is_none());

        Ok(S3Path {
            region,
            bucket,
            key,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GcsPath {
    pub bucket: String,
    pub key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GcsPathParseError {
    #[error("Not a gcp path")]
    NoPath,
    #[error("GCP path must be in the format `gs://{{bucket name}}/{{optional key prefix}}`")]
    InvalidFormat,
}

impl GcsPath {
    /// Returns `self`, possibly adding '/' at the end of the key to ensure it can be combined with another path as a directory prefix.
    pub fn ensure_directory_prefix(mut self) -> Self {
        if !self.key.is_empty() && !self.key.ends_with('/') {
            self.key.push('/');
        }
        self
    }
}

impl Display for GcsPath {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "gs://{}/{}", self.bucket, self.key)
    }
}

impl FromStr for GcsPath {
    type Err = GcsPathParseError;

    fn from_str(s: &str) -> Result<Self, GcsPathParseError> {
        let bucket_and_prefix = s.strip_prefix("gs://").ok_or(GcsPathParseError::NoPath)?;

        let mut components = bucket_and_prefix
            .splitn(2, '/')
            .take_while(|s| !s.is_empty());
        let bucket = components
            .next()
            .ok_or(GcsPathParseError::InvalidFormat)?
            .to_owned();
        let key = components.next().map(|s| s.to_owned()).unwrap_or_default();
        assert!(components.next().is_none());

        Ok(GcsPath { bucket, key })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StoragePath {
    GcsPath(GcsPath),
    S3Path(S3Path),
    LocalPath(PathBuf),
}

impl FromStr for StoragePath {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<StoragePath> {
        match S3Path::from_str(s) {
            Err(S3PathParseError::NoPath) => {}
            p => return Ok(StoragePath::S3Path(p.context("parsing an S3 path")?)),
        }

        match GcsPath::from_str(s) {
            Err(GcsPathParseError::NoPath) => {}
            p => return Ok(StoragePath::GcsPath(p.context("parsing a GCS path")?)),
        }

        Ok(StoragePath::LocalPath(s.into()))
    }
}

impl<'de> Deserialize<'de> for StoragePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ManifestKind {
    IngestorGlobal,
    IngestorSpecific,
    DataShareProcessorGlobal,
    DataShareProcessorSpecific,
    PortalServerGlobal,
}

impl FromStr for ManifestKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<ManifestKind> {
        match s {
            "ingestor-global" => Ok(ManifestKind::IngestorGlobal),
            "ingestor-specific" => Ok(ManifestKind::IngestorSpecific),
            "data-share-processor-global" => Ok(ManifestKind::DataShareProcessorGlobal),
            "data-share-processor-specific" => Ok(ManifestKind::DataShareProcessorSpecific),
            "portal-global" => Ok(ManifestKind::PortalServerGlobal),
            _ => Err(anyhow!(format!("unrecognized manifest kind {}", s))),
        }
    }
}

impl Display for ManifestKind {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            ManifestKind::IngestorGlobal => write!(f, "ingestor-global"),
            ManifestKind::IngestorSpecific => write!(f, "ingestor-specific"),
            ManifestKind::DataShareProcessorGlobal => write!(f, "data-share-processor-global"),
            ManifestKind::DataShareProcessorSpecific => write!(f, "data-share-processor-specific"),
            ManifestKind::PortalServerGlobal => write!(f, "portal-global"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TaskQueueKind {
    GcpPubSub,
    AwsSqs,
}

impl FromStr for TaskQueueKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<TaskQueueKind> {
        match s {
            "gcp-pubsub" => Ok(TaskQueueKind::GcpPubSub),
            "aws-sqs" => Ok(TaskQueueKind::AwsSqs),
            _ => Err(anyhow!(format!("unrecognized task queue kind {}", s))),
        }
    }
}

impl Display for TaskQueueKind {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            TaskQueueKind::GcpPubSub => write!(f, "gcp-pubsub"),
            TaskQueueKind::AwsSqs => write!(f, "aws-sqs"),
        }
    }
}

/// We need to be able to give &'static strs to `clap`, but sometimes we want to generate them
/// with format!(), which generates a String. This leaks a String in order to give us a &'static str.
pub fn leak_string(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// The string "-input" or "-output", for appending to arg names.
pub enum InOut {
    Input,
    Output,
}

impl InOut {
    pub fn str(&self) -> &'static str {
        match self {
            InOut::Input => "-input",
            InOut::Output => "-output",
        }
    }
}

/// One of the organizations participating in the Prio system.
pub enum Entity {
    Ingestor,
    Peer,
    Own,
    Facilitator,
    Portal,
}

impl Entity {
    pub fn str(&self) -> &'static str {
        match self {
            Entity::Ingestor => "ingestor",
            Entity::Peer => "peer",
            Entity::Own => "own",
            Entity::Facilitator => "facilitator",
            Entity::Portal => "portal",
        }
    }

    /// Return the lowercase name of this entity, plus a suffix.
    /// Intentionally leak the resulting string so it can be used
    /// as a &'static str by clap.
    pub fn suffix(&self, s: &str) -> &'static str {
        leak_string(format!("{}{}", self.str(), s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use rusoto_core::Region;
    use serde_test::{assert_de_tokens, Token};
    use std::str::FromStr;

    #[test]
    fn identity() {
        assert!(Identity::from_str("").unwrap().as_str().is_none());

        assert_eq!(
            Identity::from_str("identity").unwrap().as_str(),
            Some("identity")
        );
    }

    #[test]
    fn deserialize_identity() {
        assert_de_tokens(&Identity::from_str("").unwrap(), &[Token::Str("")]);
        assert_de_tokens(
            &Identity::from_str("identity").unwrap(),
            &[Token::Str("identity")],
        );
    }

    #[test]
    fn parse_s3path() {
        // All components are parsed properly
        let p = S3Path::from_str("s3://us-west-2/my-bucket/path/to/object").unwrap();
        assert_eq!(p.region, Region::UsWest2);
        assert_eq!(p.bucket, "my-bucket");
        assert_eq!(p.key, "path/to/object");
    }

    #[test]
    fn parse_s3path_no_key() {
        // URL with no key
        let p1 = S3Path::from_str("s3://us-west-2/my-bucket").unwrap();
        let p2 = S3Path::from_str("s3://us-west-2/my-bucket/").unwrap();
        assert_eq!(p1.key, "");
        assert_eq!(p1, p2);
    }

    #[test]
    fn parse_s3_invalid_paths() {
        // Missing region
        let e = S3Path::from_str("s3://").unwrap_err();
        assert_matches!(e, S3PathParseError::InvalidFormat);
        // Missing bucket name
        let e = S3Path::from_str("s3://us-west-2").unwrap_err();
        assert_matches!(e, S3PathParseError::InvalidFormat);
        // Empty bucket name
        let e = S3Path::from_str("s3://us-west-2/").unwrap_err();
        assert_matches!(e, S3PathParseError::InvalidFormat);
        // Invalid region
        let e = S3Path::from_str("s3://non-existent-region/my-bucket").unwrap_err();
        assert_matches!(e, S3PathParseError::InvalidRegion(_));
        // Not a path
        let e = S3Path::from_str("http://localhost").unwrap_err();
        assert_matches!(e, S3PathParseError::NoPath);
    }

    #[test]
    fn s3path_ensure_prefix() {
        let p = S3Path::from_str("s3://us-west-2/my-bucket/key_prefix").unwrap();
        let p = p.ensure_directory_prefix();
        assert_eq!(p.key, "key_prefix/");
    }

    #[test]
    fn deserialize_storagepath_s3path() {
        assert_de_tokens(
            &StoragePath::S3Path("s3://us-west-2/my-bucket".parse().unwrap()),
            &[Token::Str("s3://us-west-2/my-bucket")],
        );
    }

    #[test]
    fn deserialize_storagepath_localpath() {
        assert_de_tokens(
            &StoragePath::LocalPath("relative/path/".into()),
            &[Token::Str("relative/path/")],
        );
        assert_de_tokens(
            &StoragePath::LocalPath("/absolute/path".into()),
            &[Token::Str("/absolute/path")],
        );
    }

    #[test]
    fn parse_gcspath() {
        let p1 = GcsPath::from_str("gs://the-bucket/path/to/object").unwrap();
        assert_eq!(p1.bucket, "the-bucket");
        assert_eq!(p1.key, "path/to/object");
    }

    #[test]
    fn parse_gcspath_no_key() {
        let p1 = GcsPath::from_str("gs://the-bucket").unwrap();
        let p2 = GcsPath::from_str("gs://the-bucket/").unwrap();
        assert_eq!(p1.key, "");
        assert_eq!(p1, p2);
    }

    #[test]
    fn parse_gcs_invalid_paths() {
        // no bucket name
        let e = GcsPath::from_str("gs://").unwrap_err();
        assert_matches!(e, GcsPathParseError::InvalidFormat);
        // wrong scheme
        let e = GcsPath::from_str("s3://bucket-name/key").unwrap_err();
        assert_matches!(e, GcsPathParseError::NoPath);
    }

    #[test]
    fn gcspath_ensure_prefix() {
        let p = GcsPath::from_str("gs://the-bucket/key-prefix").unwrap();
        let p = p.ensure_directory_prefix();
        assert_eq!(p.key, "key-prefix/");
    }

    #[test]
    fn entity_suffix() {
        let val = Entity::Peer.suffix(InOut::Input.str());
        assert_eq!(val, "peer-input");
    }
}
