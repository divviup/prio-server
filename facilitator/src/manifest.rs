use crate::config::StoragePath;
use crate::http;
use anyhow::{anyhow, Context, Result};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use serde::Deserialize;
use std::{collections::HashMap, str::FromStr};

// See discussion in SpecificManifest::batch_signing_public_key
const ECDSA_P256_SPKI_PREFIX: &[u8] = &[
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// A set of batch signing public keys as might be found in a server's global
/// or specific manifest. The keys are key identifiers and the values are public
/// keys which may be used to verify batch signatures.
pub type BatchSigningPublicKeys = HashMap<String, UnparsedPublicKey<Vec<u8>>>;

/// Represents the description of a batch signing public key in a specific
/// manifest.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct BatchSigningPublicKey {
    /// The PEM-armored base64 encoding of the ASN.1 encoding of the PKIX
    /// SubjectPublicKeyInfo structure of an ECDSA P256 key.
    public_key: String,
    /// The ISO 8601 encoded UTC date at which this key expires.
    expiration: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct PacketEncryptionCertificateSigningRequest {
    /// The PEM-armored base64 encoding of the ASN.1 encoding of a PKCS#10
    /// certificate signing request containing an ECDSA P256 key.
    certificate_signing_request: String,
}

/// Represents a global manifest advertised by a data share processor. See the
/// design document for the full specification.
/// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct DataShareProcessorGlobalManifest {
    /// Format version of the manifest. Versions besides the currently supported
    /// one are rejected.
    format: u32,
    /// Identity used by the data share processor instances to access peer
    /// cloud resources
    server_identity: DataShareProcessorServerIdentity,
}

/// Represents the server-identity map inside a data share processor global
/// manifest.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct DataShareProcessorServerIdentity {
    /// The numeric account ID of the AWS account this data share processor will
    /// use to access peer cloud resources.
    aws_account_id: u64,
    /// The email address of the GCP service account this data share processor
    /// will use to access peer cloud resources.
    gcp_service_account_email: String,
}

impl DataShareProcessorGlobalManifest {
    /// Loads the global manifest relative to the provided base path and returns
    /// it. Returns an error if the manifest could not be loaded or parsed.
    pub fn from_https(base_path: &str) -> Result<Self> {
        let manifest_url = format!("{}/global-manifest.json", base_path);
        DataShareProcessorGlobalManifest::from_slice(fetch_manifest(&manifest_url)?.as_bytes())
    }

    /// Loads the manifest from the provided String. Returns an error if
    /// the manifest could not be parsed.
    pub fn from_slice(json: &[u8]) -> Result<Self> {
        let manifest: Self =
            serde_json::from_slice(json).context("failed to decode JSON global manifest")?;
        if manifest.format != 0 {
            return Err(anyhow!("unsupported manifest format {}", manifest.format));
        }
        Ok(manifest)
    }
}

/// Represents a specific manifest, used to exchange configuration parameters
/// with peer data share processors. See the design document for the full
/// specification.
/// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SpecificManifest {
    /// Format version of the manifest. Versions besides the currently supported
    /// one are rejected.
    format: u32,
    /// URL of the ingestion bucket owned by this data share processor, which
    /// may be in the form "s3://{region}/{name}" or "gs://{name}".
    ingestion_bucket: String,
    /// The ARN of the AWS IAM role that should be assumed by an ingestion
    /// server to write to this data share processor's ingestion bucket, if the
    /// ingestor does not have an AWS account of their own. This will not be
    /// present if the data share processor's ingestion bucket is not in AWS S3.
    ingestion_identity: Option<String>,
    /// URL of the validation bucket owned by this data share processor, which
    /// may be in the form "s3://{region}/{name}" or "gs://{name}".
    peer_validation_bucket: String,
    /// Keys used by this data share processor to sign batches.
    batch_signing_public_keys: HashMap<String, BatchSigningPublicKey>,
    /// Certificate signing requests containing public keys that should be used
    /// to encrypt ingestion share packets intended for this data share
    /// processor.
    packet_encryption_keys: HashMap<String, PacketEncryptionCertificateSigningRequest>,
}

impl SpecificManifest {
    /// Load the specific manifest for the specified peer relative to the
    /// provided base path. Returns an error if the manifest could not be
    /// downloaded or parsed.
    pub fn from_https(base_path: &str, peer_name: &str) -> Result<Self> {
        let manifest_url = format!("{}/{}-manifest.json", base_path, peer_name);
        SpecificManifest::from_slice(fetch_manifest(&manifest_url)?.as_bytes())
    }

    /// Loads the manifest from the provided String. Returns an error if
    /// the manifest could not be parsed.
    pub fn from_slice(json: &[u8]) -> Result<Self> {
        let manifest: Self =
            serde_json::from_slice(json).context("failed to decode JSON global manifest")?;
        if manifest.format != 1 {
            return Err(anyhow!("unsupported manifest format {}", manifest.format));
        }
        Ok(manifest)
    }

    /// Attempts to parse the values in this manifest's
    /// batch-signing-public-keys field as PEM encoded SubjectPublicKeyInfo
    /// structures containing ECDSA P256 keys, and returns a map of key
    /// identifier to the public keys on success, or an error otherwise.
    pub fn batch_signing_public_keys(&self) -> Result<BatchSigningPublicKeys> {
        let mut keys = HashMap::new();
        for (identifier, public_key) in self.batch_signing_public_keys.iter() {
            keys.insert(
                identifier.clone(),
                public_key_from_pem(&public_key.public_key)?,
            );
        }
        Ok(keys)
    }

    /// Returns the StoragePath for the data share processor's validation
    /// bucket.
    pub fn validation_bucket(&self) -> Result<StoragePath> {
        // For the time being, the path is assumed to be an S3 bucket.
        StoragePath::from_str(&self.peer_validation_bucket)
    }

    /// Returns true if all the members of the parsed manifest are valid, false
    /// otherwise.
    pub fn validate(&self) -> Result<()> {
        self.batch_signing_public_keys()
            .context("bad manifest: public keys")?;
        self.validation_bucket()
            .context("bad manifest: valiation bucket")?;
        StoragePath::from_str(&self.ingestion_bucket).context("bad manifest: ingestion bucket")?;
        Ok(())
    }
}

/// Represents the server-identity structure within an ingestion server global
/// manifest. One of aws_iam_entity or google_service_account should be Some.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct IngestionServerIdentity {
    /// The ARN of the AWS IAM entity that this ingestion server uses to access
    /// ingestion buckets,
    aws_iam_entity: Option<String>,
    /// The numeric identifier of the GCP service account that this ingestion
    /// server uses to authenticate via OIDC identity federation to access
    /// ingestion buckets. While this field's value is a number, facilitator
    /// treats it as an opaque string.
    gcp_service_account_id: Option<String>,
    /// The email address of the GCP service account that this ingestion server
    /// uses to authenticate to GCS to access ingestion buckets.
    gcp_service_account_email: String,
}

/// Represents an ingestion server's manifest. This could be a global manifest
/// or a locality-specific manifest.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct IngestionServerManifest {
    /// Format version of the manifest. Versions besides the currently supported
    /// one are rejected.
    format: u32,
    /// The identity used by the ingestor to authenticate when writing to
    /// ingestion buckets.
    server_identity: IngestionServerIdentity,
    /// ECDSA P256 public keys used by the ingestor to sign ingestion batches.
    /// The keys in this dictionary should match key_identifier values in
    /// PrioBatchSignatures received from the ingestor.
    batch_signing_public_keys: HashMap<String, BatchSigningPublicKey>,
}

impl IngestionServerManifest {
    /// Loads the global manifest relative to the provided base path and returns
    /// it. First tries to load a global manifest, then falls back to a specific
    /// manifest for the specified locality. Returns an error if no manifest
    /// could be found at either location, or if either was unparseable.
    pub fn from_https(base_path: &str, locality: Option<&str>) -> Result<Self> {
        IngestionServerManifest::from_http(base_path, locality, fetch_manifest)
    }

    fn from_http(
        base_path: &str,
        locality: Option<&str>,
        fetcher: ManifestFetcher,
    ) -> Result<Self> {
        match fetcher(&format!("{}/global-manifest.json", base_path)) {
            Ok(body) => IngestionServerManifest::from_slice(body.as_bytes()),
            Err(err) => match locality {
                Some(locality) => IngestionServerManifest::from_slice(
                    fetcher(&format!("{}/{}-manifest.json", base_path, locality))?.as_bytes(),
                ),
                None => Err(err),
            },
        }
    }

    /// Loads the manifest from the provided String. Returns an error if
    /// the manifest could not be parsed.
    pub fn from_slice(json: &[u8]) -> Result<Self> {
        let manifest: Self =
            serde_json::from_slice(json).context("failed to decode JSON manifest")?;
        if manifest.format != 1 {
            return Err(anyhow!("unsupported manifest format {}", manifest.format));
        }
        Ok(manifest)
    }

    /// Attempts to parse the values in this manifest's
    /// batch-signing-public-keys field as PEM encoded SubjectPublicKeyInfo
    /// structures containing ECDSA P256 keys, and returns a map of key
    /// identifier to the public keys on success, or an error otherwise.
    pub fn batch_signing_public_keys(&self) -> Result<BatchSigningPublicKeys> {
        let mut keys = HashMap::new();
        for (identifier, public_key) in self.batch_signing_public_keys.iter() {
            keys.insert(
                identifier.clone(),
                public_key_from_pem(&public_key.public_key)?,
            );
        }
        Ok(keys)
    }

    /// Returns true if all the members of the parsed manifest are valid, false
    /// otherwise.
    pub fn validate(&self) -> Result<()> {
        self.batch_signing_public_keys()
            .context("bad manifest: public keys")?;
        Ok(())
    }
}

/// Represents the global manifest for a portal server.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct PortalServerGlobalManifest {
    /// Format version of the manifest. Versions besides the currently supported
    /// one are rejected.
    format: u32,
    /// URL of the bucket to which facilitator servers should write sum parts,
    /// which may be in the form "s3://{region}/{name}" or "gs://{name}".
    facilitator_sum_part_bucket: String,
    /// URL of the bucket to which PHA servers should write sum parts, which may
    /// be in the form "s3://{region}/{name}" or "gs://{name}".
    pha_sum_part_bucket: String,
}

impl PortalServerGlobalManifest {
    pub fn from_https(base_path: &str) -> Result<Self> {
        let manifest_url = format!("{}/global-manifest.json", base_path);
        PortalServerGlobalManifest::from_slice(fetch_manifest(&manifest_url)?.as_bytes())
    }

    /// Loads the manifest from the provided String. Returns an error if
    /// the manifest could not be parsed.
    pub fn from_slice(json: &[u8]) -> Result<Self> {
        let manifest: PortalServerGlobalManifest =
            serde_json::from_slice(json).context("failed to decode JSON global manifest")?;
        if manifest.format != 1 {
            return Err(anyhow!("unsupported manifest format {}", manifest.format));
        }
        Ok(manifest)
    }

    /// Returns the StoragePath for this portal server, returning the PHA bucket
    /// if is_pha is true, or the facilitator bucket otherwise.
    pub fn sum_part_bucket(&self, is_pha: bool) -> Result<StoragePath> {
        // For now, the path is assumed to be a GCS bucket.
        StoragePath::from_str(if is_pha {
            &self.pha_sum_part_bucket
        } else {
            &self.facilitator_sum_part_bucket
        })
    }

    /// Returns true if all the members of the parsed manifest are valid, false
    /// otherwise.
    pub fn validate(&self) -> Result<()> {
        self.sum_part_bucket(true)
            .context("bad manifest: pha sum part bucket")?;
        self.sum_part_bucket(false)
            .context("bad manifest: facilitator sum part bucket")?;
        Ok(())
    }
}

/// A function that fetches a manifest from the provided URL, returning the
/// manifest body as a String on success.
type ManifestFetcher = fn(&str) -> Result<String>;

/// Obtains a manifest file from the provided URL, returning an error if the URL
/// is not https or if a problem occurs during the transfer.
fn fetch_manifest(manifest_url: &str) -> Result<String> {
    if !manifest_url.starts_with("https://") {
        return Err(anyhow!("Manifest must be fetched over HTTPS"));
    }
    http::simple_get_request(
        url::Url::parse(manifest_url)
            .context(format!("failed to parse manifest url: {}", manifest_url))?,
    )
}

/// Attempts to parse the provided string as a PEM encoded PKIX
/// SubjectPublicKeyInfo structure containing an ECDSA P256 public key, and
/// returns an UnparsedPublicKey containing that key on success.
fn public_key_from_pem(pem_key: &str) -> Result<UnparsedPublicKey<Vec<u8>>> {
    // No Rust crate that we have found gives us an easy way to parse PKIX
    // SubjectPublicKeyInfo structures to get at the public key which can
    // then be used in ring::signature. Since we know the keys we deal with
    // should always be ECDSA P256, we can instead check that the binary
    // blob inside the PEM has the expected prefix for this kind of key in
    // this kind of encoding, as suggested in this GitHub issue on ring:
    // https://github.com/briansmith/ring/issues/881
    if pem_key.is_empty() {
        return Err(anyhow!("empty PEM input"));
    }
    let pem = pem::parse(&pem_key).context(format!("failed to parse key as PEM: {}", pem_key))?;
    if pem.tag != "PUBLIC KEY" {
        return Err(anyhow!(
            "key for identifier {} is not a PEM encoded public key"
        ));
    }

    // An ECDSA P256 public key in this encoding will always be 26 bytes of
    // prefix + 65 bytes of key = 91 bytes total. e.g.,
    // https://lapo.it/asn1js/#MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgD______________________________________________________________________________________w
    if pem.contents.len() != 91 {
        return Err(anyhow!(
            "PEM contents are wrong size for ASN.1 encoded ECDSA P256 SubjectPublicKeyInfo"
        ));
    }

    let (prefix, key) = pem.contents.split_at(ECDSA_P256_SPKI_PREFIX.len());

    if prefix != ECDSA_P256_SPKI_PREFIX {
        return Err(anyhow!(
            "PEM contents are not ASN.1 encoded ECDSA P256 SubjectPublicKeyInfo"
        ));
    }

    Ok(UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_ASN1,
        Vec::from(key),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GCSPath, S3Path};
    use crate::test_utils::{
        default_ingestor_private_key, DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO,
    };
    use ring::rand::SystemRandom;
    use rusoto_core::Region;
    use url::Url;

    fn url_fetcher(url: &str) -> Result<String> {
        http::simple_get_request(Url::parse(url)?)
    }

    #[test]
    fn load_data_share_processor_global_manifest() {
        let json = br#"
{
    "format": 0,
    "server-identity": {
        "aws-account-id": 12345678901234567,
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com"
    }
}
            "#;
        let manifest = DataShareProcessorGlobalManifest::from_slice(json).unwrap();
        assert_eq!(manifest.format, 0);
        assert_eq!(
            manifest.server_identity,
            DataShareProcessorServerIdentity {
                aws_account_id: 12345678901234567,
                gcp_service_account_email: "service-account@project-name.iam.gserviceaccount.com"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn invalid_data_share_processor_global_manifests() {
        let invalid_manifests: Vec<&str> = vec![
            // no format key
            r#"
{
    "server-identity": {
        "aws-account-id": 12345678901234567,
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com"
    }
}
        "#,
            // wrong format version
            r#"
 {
    "format": 2,
    "server-identity": {
        "aws-account-id": 12345678901234567,
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com"
    }
}
        "#,
            // no server identity
            r#"
 {
    "format": 0
}
        "#,
            // missing aws account
            r#"
 {
    "format": 0,
    "server-identity": {
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com"
    }
}
        "#,
            // non numeric aws account
            r#"
 {
    "format": 0,
    "server-identity": {
        "aws-account-id": "not-a-number",
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com"
    }
}
        "#,
            // missing GCP email
            r#"
 {
    "format": 0,
    "server-identity": {
        "aws-account-id": 12345678901234567
    }
}
        "#,
            // non-string GCP email
            r#"
 {
    "format": 0,
    "server-identity": {
        "aws-account-id": 12345678901234567,
        "gcp-service-account-email": 14
    }
}
        "#,
            // unexpected top-level field
            r#"
{
    "format": 0,
    "server-identity": {
        "aws-account-id": 12345678901234567,
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com"
    },
    "unexpected": "some value"
}
        "#,
            // unexpected server-identity field
            r#"
{
    "format": 0,
    "server-identity": {
        "aws-account-id": 12345678901234567,
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com",
        "unexpected": "some value"
    }
}
        "#,
        ];

        for invalid_manifest in &invalid_manifests {
            DataShareProcessorGlobalManifest::from_slice(invalid_manifest.as_bytes()).unwrap_err();
        }
    }

    #[test]
    fn load_specific_manifest() {
        let json = format!(
            r#"
{{
    "format": 1,
    "packet-encryption-keys": {{
        "fake-key-1": {{
            "certificate-signing-request": "who cares"
        }}
    }},
    "batch-signing-public-keys": {{
        "fake-key-2": {{
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n"
      }}
    }},
    "ingestion-bucket": "s3://us-west-1/ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "gs://validation/path/fragment"
}}
    "#,
            DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO
        );
        let manifest = SpecificManifest::from_slice(json.as_bytes()).unwrap();

        let mut expected_batch_keys = HashMap::new();
        expected_batch_keys.insert(
            "fake-key-2".to_owned(),
            BatchSigningPublicKey {
                expiration: "".to_string(),
                public_key: format!(
                    "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
                    DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO
                ),
            },
        );
        let mut expected_packet_encryption_csrs = HashMap::new();
        expected_packet_encryption_csrs.insert(
            "fake-key-1".to_owned(),
            PacketEncryptionCertificateSigningRequest {
                certificate_signing_request: "who cares".to_owned(),
            },
        );
        let expected_manifest = SpecificManifest {
            format: 1,
            batch_signing_public_keys: expected_batch_keys,
            packet_encryption_keys: expected_packet_encryption_csrs,
            ingestion_bucket: "s3://us-west-1/ingestion".to_string(),
            ingestion_identity: Some("arn:aws:iam:something:fake".to_owned()),
            peer_validation_bucket: "gs://validation/path/fragment".to_string(),
        };
        assert_eq!(manifest, expected_manifest);
        let batch_signing_keys = manifest.batch_signing_public_keys().unwrap();
        let content = b"some content";
        let signature = default_ingestor_private_key()
            .key
            .sign(&SystemRandom::new(), content)
            .unwrap();
        batch_signing_keys
            .get("fake-key-2")
            .unwrap()
            .verify(content, signature.as_ref())
            .unwrap();

        if let StoragePath::GCSPath(path) = manifest.validation_bucket().unwrap() {
            assert_eq!(
                path,
                GCSPath {
                    bucket: "validation".to_owned(),
                    key: "path/fragment".to_owned(),
                }
            );
        } else {
            assert!(false, "unexpected storage path type");
        }

        manifest.validate().unwrap();
    }

    #[test]
    fn invalid_specific_manifest() {
        let invalid_manifests = vec![
            "not-json",
            "{ \"missing\": \"keys\"}",
            // No format key
            r#"
{
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "s3://us-west-1/ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "gs://validation"
}
    "#,
            // Format key with wrong value
            r#"
{
    "format": 0,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "s3://us-west-1/ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "gs://validation"
}
    "#,
            // Format key with wrong type
            r#"
{
    "format": "zero",
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "gs://ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "s3://us-west-1/validation"
}
    "#,
            // Role ARN with wrong type
            r#"
{
    "format": 1,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "ingestion-identity": 1,
    "peer-validation-bucket": "us-west-1/validation"
}
"#,
            // Unexpected top-level field
            r#"
{
    "format": 1,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "s3://us-west-1/ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "gs://validation",
    "unexpected": "some value"
}
"#,
            // Unexpected BatchSigningPublicKey field
            r#"
{
    "format": 1,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----",
        "unexpected": "some value"
      }
    },
    "ingestion-bucket": "s3://us-west-1/ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "gs://validation"
}
"#,
            // Unexpected PacketEncryptionCertificateSigningRequest field
            r#"
{
    "format": 1,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares",
            "unexpected": "some value"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "s3://us-west-1/ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "gs://validation"
}
"#,
        ];

        for invalid_manifest in &invalid_manifests {
            SpecificManifest::from_slice(invalid_manifest.as_bytes()).unwrap_err();
        }
    }

    #[test]
    fn invalid_specific_public_key() {
        let manifests_with_invalid_public_keys = vec![
            // Wrong PEM block
            r#"
{
    "format": 1,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN EC PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEIKh3MccE1cdSF4pnEb+U0MmGYfkoQzOl2aiaJ6D9ZudqDdGiyA9YSUq3yia56nYJh5mk+HlzTX+AufoNR2bfrg==\n-----END EC PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "gs://ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "s3://us-west-1/validation"
}
    "#,
            // PEM contents not an ASN.1 SPKI
            r#"
{
    "format": 1,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nBIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "gs://ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "s3://us-west-1/validation"
}
    "#,
            // PEM contents too short
            r#"
{
    "format": 1,
    "packet-encryption-keys": {
        "fake-key-1": {
            "certificate-signing-request": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\ndG9vIHNob3J0Cg==\n-----END PUBLIC KEY-----\n"
      }
    },
    "ingestion-bucket": "gs://ingestion",
    "ingestion-identity": "arn:aws:iam:something:fake",
    "peer-validation-bucket": "s3://us-west-1/validation"
}
    "#,
        ];
        for invalid_manifest in &manifests_with_invalid_public_keys {
            let manifest = SpecificManifest::from_slice(invalid_manifest.as_bytes()).unwrap();
            assert!(manifest.batch_signing_public_keys().is_err());
        }
    }

    #[test]
    fn load_ingestor_manifest() {
        let manifest_with_aws_identity = r#"
{
    "format": 1,
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-id": "12345678901234567890",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/8OzWHOvmin1KeaiMWFQXfNwS9uZ\n839EjwMff1VB4dnurW38FRP+Z0KxIdvvrPsGMWdPXoTASRAPEHHqpWlTlg==\n-----END PUBLIC KEY-----\n",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
            "#;
        let manifest_with_gcp_identity = r#"
{
    "format": 1,
    "server-identity": {
        "gcp-service-account-id": "112310747466759665351",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-2": {
            "public-key": "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/8OzWHOvmin1KeaiMWFQXfNwS9uZ\n839EjwMff1VB4dnurW38FRP+Z0KxIdvvrPsGMWdPXoTASRAPEHHqpWlTlg==\n-----END PUBLIC KEY-----\n",
            "expiration": "2021-01-15T18:53:20Z"
        },
        "another-key": {
            "public-key": "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/8OzWHOvmin1KeaiMWFQXfNwS9uZ\n839EjwMff1VB4dnurW38FRP+Z0KxIdvvrPsGMWdPXoTASRAPEHHqpWlTlg==\n-----END PUBLIC KEY-----\n",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
            "#;

        let manifest =
            IngestionServerManifest::from_slice(manifest_with_aws_identity.as_bytes()).unwrap();
        assert_eq!(
            manifest.server_identity.aws_iam_entity,
            Some("arn:aws:iam::338276578713:role/ingestor-1-role".to_owned())
        );
        assert_eq!(
            manifest.server_identity.gcp_service_account_id,
            Some("12345678901234567890".to_owned())
        );
        assert_eq!(
            manifest.server_identity.gcp_service_account_email,
            "foo@bar.com".to_owned()
        );
        let batch_signing_public_keys = manifest.batch_signing_public_keys().unwrap();
        batch_signing_public_keys.get("key-identifier-1").unwrap();
        assert!(batch_signing_public_keys.get("nosuchkey").is_none());

        let manifest =
            IngestionServerManifest::from_slice(manifest_with_gcp_identity.as_bytes()).unwrap();
        assert_eq!(manifest.server_identity.aws_iam_entity, None);
        assert_eq!(
            manifest.server_identity.gcp_service_account_email,
            "foo@bar.com".to_owned()
        );
        assert_eq!(
            manifest.server_identity.gcp_service_account_id,
            Some("112310747466759665351".to_owned())
        );
        let batch_signing_public_keys = manifest.batch_signing_public_keys().unwrap();
        batch_signing_public_keys.get("key-identifier-2").unwrap();
        assert!(batch_signing_public_keys.get("nosuchkey").is_none());
    }

    #[test]
    fn invalid_ingestor_global_manifest() {
        let invalid_manifests = vec![
            "not-json",
            "{ \"missing\": \"keys\"}",
            // No format key
            r#"
{
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "----BEGIN PUBLIC KEY----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEI3MQm+HzXvaYa2mVlhB4zknbtAT8cSxakmBoJcBKGqGw\nYS0bhxSpuvABM1kdBTDpQhXnVdcq+LSiukXJRpGHVg==\n----END PUBLIC KEY----",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
    "#,
            // Format key with wrong value
            r#"
{
    "format": 2,
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "----BEGIN PUBLIC KEY----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEI3MQm+HzXvaYa2mVlhB4zknbtAT8cSxakmBoJcBKGqGw\nYS0bhxSpuvABM1kdBTDpQhXnVdcq+LSiukXJRpGHVg==\n----END PUBLIC KEY----",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
    "#,
            // Format key with wrong type
            r#"
{
    "format": "zero",
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "----BEGIN PUBLIC KEY----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEI3MQm+HzXvaYa2mVlhB4zknbtAT8cSxakmBoJcBKGqGw\nYS0bhxSpuvABM1kdBTDpQhXnVdcq+LSiukXJRpGHVg==\n----END PUBLIC KEY----",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
    "#,
            // Unexpected top-level field
            r#"
{
    "format": 1,
    "unexpected": "some value",
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "----BEGIN PUBLIC KEY----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEI3MQm+HzXvaYa2mVlhB4zknbtAT8cSxakmBoJcBKGqGw\nYS0bhxSpuvABM1kdBTDpQhXnVdcq+LSiukXJRpGHVg==\n----END PUBLIC KEY----",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
    "#,
            // Unexpected server-identity field
            r#"
{
    "format": 1,
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-email": "foo@bar.com",
        "unexpected": "some value"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "----BEGIN PUBLIC KEY----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEI3MQm+HzXvaYa2mVlhB4zknbtAT8cSxakmBoJcBKGqGw\nYS0bhxSpuvABM1kdBTDpQhXnVdcq+LSiukXJRpGHVg==\n----END PUBLIC KEY----",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
    "#,
        ];

        for invalid_manifest in &invalid_manifests {
            IngestionServerManifest::from_slice(invalid_manifest.as_bytes()).unwrap_err();
        }
    }

    #[test]
    fn load_portal_global_manifest() {
        let manifest = r#"
{
    "format": 1,
    "facilitator-sum-part-bucket": "gs://facilitator-bucket",
    "pha-sum-part-bucket": "s3://us-west-1/pha-bucket"
}
            "#;

        let manifest = PortalServerGlobalManifest::from_slice(manifest.as_bytes()).unwrap();
        if let StoragePath::GCSPath(path) = manifest.sum_part_bucket(false).unwrap() {
            assert_eq!(
                path,
                GCSPath {
                    bucket: "facilitator-bucket".to_owned(),
                    key: "".to_owned(),
                }
            );
        } else {
            assert!(false, "unexpected storage path type");
        }
        if let StoragePath::S3Path(path) = manifest.sum_part_bucket(true).unwrap() {
            assert_eq!(
                path,
                S3Path {
                    region: Region::UsWest1,
                    bucket: "pha-bucket".to_owned(),
                    key: "".to_owned(),
                }
            );
        } else {
            assert!(false, "unexpected storage path type");
        }
    }

    #[test]
    fn invalid_portal_global_manifests() {
        let invalid_manifests = vec![
            "not-json",
            "{ \"missing\": \"keys\"}",
            // No format key
            r#"
{
    "facilitator-sum-part-bucket": "gs://facilitator-bucket",
    "pha-sum-part-bucket": "gs://pha-bucket"
}
    "#,
            // Format key with wrong value
            r#"
{
    "format": 0,
    "facilitator-sum-part-bucket": "gs://facilitator-bucket",
    "pha-sum-part-bucket": "gs://pha-bucket"
}
    "#,
            // Format key with wrong type
            r#"
{
    "format": "zero",
    "facilitator-sum-part-bucket": "gs://facilitator-bucket",
    "pha-sum-part-bucket": "gs://pha-bucket"
}
    "#,
            // Missing field
            r#"
{
    "format": 1,
    "facilitator-sum-part-bucket": "gs://facilitator-bucket"
}
    "#,
            // Unexpected top-level field
            r#"
{
    "format": 1,
    "facilitator-sum-part-bucket": "gs://facilitator-bucket",
    "pha-sum-part-bucket": "gs://pha-bucket",
    "unexpected": "some value"
}
    "#,
        ];

        for invalid_manifest in &invalid_manifests {
            PortalServerGlobalManifest::from_slice(invalid_manifest.as_bytes()).unwrap_err();
        }
    }

    use mockito::mock;

    #[test]
    fn ingestor_global_manifest() {
        let mocked_get = mock("GET", "/global-manifest.json")
            .with_status(200)
            .with_body(r#"
{
    "format": 1,
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-id": "12345678901234567890",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/8OzWHOvmin1KeaiMWFQXfNwS9uZ\n839EjwMff1VB4dnurW38FRP+Z0KxIdvvrPsGMWdPXoTASRAPEHHqpWlTlg==\n-----END PUBLIC KEY-----\n",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
            "#)
            .expect(1)
            .create();

        IngestionServerManifest::from_http(&mockito::server_url(), None, url_fetcher).unwrap();

        mocked_get.assert();
    }

    #[test]
    fn unparseable_ingestor_global_manifest() {
        let mocked_get = mock("GET", "/global-manifest.json")
            .with_status(200)
            .with_body("invalid manifest")
            .expect(1)
            .create();

        IngestionServerManifest::from_http(&mockito::server_url(), None, url_fetcher).unwrap_err();

        mocked_get.assert();
    }

    #[test]
    fn ingestor_specific_manifest_fallback() {
        let mocked_global_get = mock("GET", "/global-manifest.json")
            .with_status(404)
            .expect(1)
            .create();

        let mocked_specific_get = mock("GET", "/instance-name-manifest.json")
        .with_status(200)
        .with_body(r#"
{
    "format": 1,
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role",
        "gcp-service-account-id": "12345678901234567890",
        "gcp-service-account-email": "foo@bar.com"
    },
    "batch-signing-public-keys": {
        "key-identifier-1": {
            "public-key": "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/8OzWHOvmin1KeaiMWFQXfNwS9uZ\n839EjwMff1VB4dnurW38FRP+Z0KxIdvvrPsGMWdPXoTASRAPEHHqpWlTlg==\n-----END PUBLIC KEY-----\n",
            "expiration": "2021-01-15T18:53:20Z"
        }
    }
}
            "#)
            .expect(1)
            .create();

        IngestionServerManifest::from_http(
            &mockito::server_url(),
            Some("instance-name"),
            url_fetcher,
        )
        .unwrap();

        mocked_global_get.assert();
        mocked_specific_get.assert();
    }

    #[test]
    fn unparseable_ingestor_specific_manifest() {
        let mocked_global_get = mock("GET", "/global-manifest.json")
            .with_status(404)
            .expect(1)
            .create();

        let mocked_specific_get = mock("GET", "/instance-name-manifest.json")
            .with_status(200)
            .with_body("invalid manifest")
            .expect(1)
            .create();

        IngestionServerManifest::from_http(
            &mockito::server_url(),
            Some("instance-name"),
            url_fetcher,
        )
        .unwrap_err();

        mocked_global_get.assert();
        mocked_specific_get.assert();
    }

    #[test]
    fn missing_ingestor_specific_manifest() {
        let mocked_global_get = mock("GET", "/global-manifest.json")
            .with_status(404)
            .expect(1)
            .create();

        let mocked_specific_get = mock("GET", "/instance-name-manifest.json")
            .with_status(404)
            .expect(1)
            .create();

        IngestionServerManifest::from_http(
            &mockito::server_url(),
            Some("instance-name"),
            url_fetcher,
        )
        .unwrap_err();

        mocked_global_get.assert();
        mocked_specific_get.assert();
    }
}
