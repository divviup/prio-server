use anyhow::{Context, Result};
use rusoto_core::{region::ParseRegionError, Region};
use serde::{de, Deserialize, Deserializer};
use std::{path::PathBuf, str::FromStr};

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

#[derive(Clone, Debug, PartialEq)]
pub enum StoragePath {
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

        Ok(StoragePath::LocalPath(s.into()))
    }
}

impl<'de> Deserialize<'de> for StoragePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{S3Path, S3PathParseError, StoragePath};
    use assert_matches::assert_matches;
    use rusoto_core::Region;
    use serde_test::{assert_de_tokens, Token};
    use std::str::FromStr;

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
}
