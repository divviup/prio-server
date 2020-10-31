use anyhow::{Context, Result};
use chrono::Duration;
use once_cell::sync::Lazy;
use regex::Regex;
use rusoto_core::{region::ParseRegionError, Region};
use serde::{de, export::Formatter, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fmt::{self, Display},
    path::PathBuf,
    str::FromStr,
};

/// Identity represents a cloud identity: Either an AWS IAM ARN (i.e. "arn:...")
/// or a GCP ServiceAccount (i.e. "foo@bar.com").
pub type Identity<'a> = Option<&'a str>;

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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
pub struct GCSPath {
    pub bucket: String,
    pub key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GCSPathParseError {
    #[error("Not a gcp path")]
    NoPath,
    #[error("GCP path must be in the format `gs://{{bucket name}}/{{optional key prefix}}`")]
    InvalidFormat,
}

impl GCSPath {
    /// Returns `self`, possibly adding '/' at the end of the key to ensure it can be combined with another path as a directory prefix.
    pub fn ensure_directory_prefix(mut self) -> Self {
        if !self.key.is_empty() && !self.key.ends_with('/') {
            self.key.push('/');
        }
        self
    }
}

impl Display for GCSPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "gs://{}/{}", self.bucket, self.key)
    }
}

impl FromStr for GCSPath {
    type Err = GCSPathParseError;

    fn from_str(s: &str) -> Result<Self, GCSPathParseError> {
        let bucket_and_prefix = s.strip_prefix("gs://").ok_or(GCSPathParseError::NoPath)?;

        let mut components = bucket_and_prefix
            .splitn(2, '/')
            .take_while(|s| !s.is_empty());
        let bucket = components
            .next()
            .ok_or(GCSPathParseError::InvalidFormat)?
            .to_owned();
        let key = components.next().map(|s| s.to_owned()).unwrap_or_default();
        assert!(components.next().is_none());

        Ok(GCSPath { bucket, key })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StoragePath {
    GCSPath(GCSPath),
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

        match GCSPath::from_str(s) {
            Err(GCSPathParseError::NoPath) => {}
            p => return Ok(StoragePath::GCSPath(p.context("parsing a GCS path")?)),
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

/// Represents a simple duration specified in terms of whole hours, minutes and seconds. Mostly used
/// for user input in flags or config files. For computations it should usually be converted to a
/// [`chrono::Duration`] using [`to_duration`](DayDuration::to_duration).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DayDuration {
    hours: u32,
    minutes: u32,
    seconds: u32,
}

impl DayDuration {
    pub fn from_hms(hours: u32, minutes: u32, seconds: u32) -> DayDuration {
        DayDuration {
            hours,
            minutes,
            seconds,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        // Components aren't allowed to overflow unless they're the first non-0 component
        if self.hours != 0 && self.minutes >= 60 {
            return Err("minutes > 59 are not allowed if hours is specified".into());
        }
        if (self.hours != 0 || self.minutes != 0) && self.seconds >= 60 {
            return Err("seconds > 59 are not allowed if hours or minutes are specified".into());
        }
        Ok(())
    }

    pub fn to_duration(&self) -> Duration {
        Duration::hours(self.hours.into())
            + Duration::minutes(self.minutes.into())
            + Duration::seconds(self.seconds.into())
    }
}

impl From<DayDuration> for Duration {
    fn from(d: DayDuration) -> Duration {
        d.to_duration()
    }
}

impl fmt::Display for DayDuration {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        if self.hours != 0 {
            write!(f, "{}h", self.hours)?;
        }
        if self.minutes != 0 {
            write!(f, "{}m", self.minutes)?;
        }
        if self.seconds != 0 || (self.hours == 0 && self.minutes == 0) {
            write!(f, "{}s", self.seconds)?;
        }
        Ok(())
    }
}

impl FromStr for DayDuration {
    type Err = String;

    fn from_str(s: &str) -> Result<DayDuration, String> {
        static RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$").unwrap());

        let groups = RE
            .captures(&s)
            .ok_or("not in expected format (e.g. 1h30m20s)")?;

        let parse_component = |group_idx, label| -> Result<u32, String> {
            groups
                .get(group_idx)
                .map_or(Ok(0), |x| u32::from_str(x.as_str()))
                .map_err(|e| format!("failed to parse {}: {}", label, e))
        };

        let d = DayDuration {
            hours: parse_component(1, "hours")?,
            minutes: parse_component(2, "minutes")?,
            seconds: parse_component(3, "seconds")?,
        };
        d.validate()?;
        Ok(d)
    }
}

impl Serialize for DayDuration {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DayDuration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DayDuration, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{GCSPath, GCSPathParseError, S3Path, S3PathParseError, StoragePath};
    use crate::config::DayDuration;
    use assert_matches::assert_matches;
    use rusoto_core::Region;
    use serde_test::{assert_de_tokens, assert_tokens, Token};
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

    #[test]
    fn dayduration_serialization() {
        let testcases = [
            // All combinations of components
            (DayDuration::from_hms(0, 0, 0), "0s"),
            (DayDuration::from_hms(11, 0, 0), "11h"),
            (DayDuration::from_hms(0, 22, 0), "22m"),
            (DayDuration::from_hms(0, 0, 33), "33s"),
            (DayDuration::from_hms(11, 22, 0), "11h22m"),
            (DayDuration::from_hms(11, 0, 33), "11h33s"),
            (DayDuration::from_hms(0, 22, 33), "22m33s"),
            (DayDuration::from_hms(11, 22, 33), "11h22m33s"),
            // Allowed overflows
            (DayDuration::from_hms(0, 0, 90), "90s"),
            (DayDuration::from_hms(0, 90, 33), "90m33s"),
            (DayDuration::from_hms(90, 22, 33), "90h22m33s"),
        ];

        for (duration, serialized) in &testcases {
            assert_tokens(duration, &[Token::Str(serialized)]);
        }
    }

    #[test]
    fn dayduration_parse_errors() {
        let testcases = [
            // Wrong format
            ("123", "not in expected format"),
            ("h", "not in expected format"),
            ("33s22m", "not in expected format"),
            ("11hXXm33s", "not in expected format"),
            // Disallowed overflow
            ("1m90s", "seconds > 59"),
            ("1h90s", "seconds > 59"),
            ("1h90m", "minutes > 59"),
            // Int parsing error (overflow)
            ("9999999999s", "failed to parse seconds"),
            ("9999999999m", "failed to parse minutes"),
            ("9999999999h", "failed to parse hours"),
        ];

        for (serialized, expected_error) in &testcases {
            match DayDuration::from_str(serialized) {
                Ok(val) => panic!(
                    "Expected {:?} to fail to deserialize, but it succeeded: {:?}",
                    serialized, val
                ),
                Err(err) if !err.contains(expected_error) => panic!(
                    "Expected {:?} to fail with {:?}, but failed with: {:?}",
                    serialized, expected_error, err
                ),
                _ => {}
            }
        }
    }

    #[test]
    fn parse_gcspath() {
        let p1 = GCSPath::from_str("gs://the-bucket/path/to/object").unwrap();
        assert_eq!(p1.bucket, "the-bucket");
        assert_eq!(p1.key, "path/to/object");
    }

    #[test]
    fn parse_gcspath_no_key() {
        let p1 = GCSPath::from_str("gs://the-bucket").unwrap();
        let p2 = GCSPath::from_str("gs://the-bucket/").unwrap();
        assert_eq!(p1.key, "");
        assert_eq!(p1, p2);
    }

    #[test]
    fn parse_gcs_invalid_paths() {
        // no bucket name
        let e = GCSPath::from_str("gs://").unwrap_err();
        assert_matches!(e, GCSPathParseError::InvalidFormat);
        // wrong scheme
        let e = GCSPath::from_str("s3://bucket-name/key").unwrap_err();
        assert_matches!(e, GCSPathParseError::NoPath);
    }

    #[test]
    fn gcspath_ensure_prefix() {
        let p = GCSPath::from_str("gs://the-bucket/key-prefix").unwrap();
        let p = p.ensure_directory_prefix();
        assert_eq!(p.key, "key-prefix/");
    }
}
