use crate::config::StoragePath;
use anyhow::{anyhow, Context, Result};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use serde::Deserialize;
use serde_json::from_reader;
use std::{collections::HashMap, io::Read, str::FromStr};
use ureq::Response;

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
#[serde(rename_all = "kebab-case")]
struct BatchSigningPublicKey {
    /// The PEM-armored base64 encoding of the ASN.1 encoding of the PKIX
    /// SubjectPublicKeyInfo structure of an ECDSA P256 key.
    public_key: String,
    /// The ISO 8601 encoded UTC date at which this key expires.
    expiration: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct PacketEncryptionCertificate {
    /// The PEM-armored base64 encoding of the ASN.1 encoding of an X.509
    /// certificate containing an ECDSA P256 key.
    certificate: String,
}

/// Represents a global manifest advertised by a data share processor. See the
/// design document for the full specification.
/// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
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
#[serde(rename_all = "kebab-case")]
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
    pub fn from_https(base_path: &str) -> Result<DataShareProcessorGlobalManifest> {
        let manifest_url = format!("{}/global-manifest.json", base_path);
        DataShareProcessorGlobalManifest::from_reader(fetch_manifest(&manifest_url)?.into_reader())
    }

    /// Loads the manifest from the provided std::io::Read. Returns an error if
    /// the manifest could not be read or parsed.
    pub fn from_reader<R: Read>(reader: R) -> Result<DataShareProcessorGlobalManifest> {
        let manifest: DataShareProcessorGlobalManifest =
            from_reader(reader).context("failed to decode JSON global manifest")?;
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
#[serde(rename_all = "kebab-case")]
pub struct SpecificManifest {
    /// Format version of the manifest. Versions besides the currently supported
    /// one are rejected.
    format: u32,
    /// Region and name of the ingestion S3 bucket owned by this data share
    /// processor.
    ingestion_bucket: String,
    /// Region and name of the peer validation S3 bucket owned by this data
    /// share processor.
    peer_validation_bucket: String,
    /// Keys used by this data share processor to sign batches.
    batch_signing_public_keys: HashMap<String, BatchSigningPublicKey>,
    /// Certificates containing public keys that should be used to encrypt
    /// ingestion share packets intended for this data share processor.
    packet_encryption_certificates: HashMap<String, PacketEncryptionCertificate>,
}

impl SpecificManifest {
    /// Load the specific manifest for the specified peer relative to the
    /// provided base path. Returns an error if the manifest could not be
    /// downloaded or parsed.
    pub fn from_https(base_path: &str, peer_name: &str) -> Result<SpecificManifest> {
        let manifest_url = format!("{}/{}-manifest.json", base_path, peer_name);
        SpecificManifest::from_reader(fetch_manifest(&manifest_url)?.into_reader())
    }

    /// Loads the manifest from the provided std::io::Read. Returns an error if
    /// the manifest could not be read or parsed.
    pub fn from_reader<R: Read>(reader: R) -> Result<SpecificManifest> {
        let manifest: SpecificManifest =
            from_reader(reader).context("failed to decode JSON specific manifest")?;
        if manifest.format != 0 {
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
        StoragePath::from_str(&format!("s3://{}", &self.peer_validation_bucket))
    }

    /// Returns true if all the members of the parsed manifest are valid, false
    /// otherwise.
    pub fn validate(&self) -> Result<()> {
        self.batch_signing_public_keys()
            .context("bad manifest: public keys")?;
        self.validation_bucket()
            .context("bad manifest: valiation bucket")?;
        StoragePath::from_str(&format!("s3://{}", &self.ingestion_bucket))
            .context("bad manifest: ingestion bucket")?;
        Ok(())
    }
}

/// Represents the server-identity structure within an ingestion server global
/// manifest. One of aws_iam_entity or google_service_account should be Some.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
struct IngestionServerIdentity {
    /// The ARN of the AWS IAM entity that this ingestion server uses to access
    /// ingestion buckets,
    aws_iam_entity: Option<String>,
    /// The numeric identifier of the GCP service account that this ingestion
    /// server uses to authenticate via OIDC identity federation to access
    /// ingestion buckets. While this field's value is a number, facilitator
    /// treats it as an opaque string.
    google_service_account: Option<String>,
}

/// Represents an ingestion server's global manifest.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct IngestionServerGlobalManifest {
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

impl IngestionServerGlobalManifest {
    /// Loads the global manifest relative to the provided base path and returns
    /// it. Returns an error if the manifest could not be loaded or parsed.
    pub fn from_https(base_path: &str) -> Result<IngestionServerGlobalManifest> {
        let manifest_url = format!("{}/global-manifest.json", base_path);
        IngestionServerGlobalManifest::from_reader(fetch_manifest(&manifest_url)?.into_reader())
    }

    /// Loads the manifest from the provided std::io::Read. Returns an error if
    /// the manifest could not be read or parsed.
    pub fn from_reader<R: Read>(reader: R) -> Result<IngestionServerGlobalManifest> {
        let manifest: IngestionServerGlobalManifest =
            from_reader(reader).context("failed to decode JSON global manifest")?;
        if manifest.format != 0 {
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
#[serde(rename_all = "kebab-case")]
pub struct PortalServerGlobalManifest {
    /// Format version of the manifest. Versions besides the currently supported
    /// one are rejected.
    format: u32,
    /// Name of the GCS bucket to which facilitator servers should write their
    /// sum part batches for aggregation by the portal server.
    facilitator_sum_part_bucket: String,
    /// Name of the GCS bucket to which PHA servers should write their sum part
    /// batches for aggregation by the portal server.
    pha_sum_part_bucket: String,
}

impl PortalServerGlobalManifest {
    pub fn from_https(base_path: &str) -> Result<PortalServerGlobalManifest> {
        let manifest_url = format!("{}/global-manifest.json", base_path);
        PortalServerGlobalManifest::from_reader(fetch_manifest(&manifest_url)?.into_reader())
    }

    /// Loads the manifest from the provided std::io::Read. Returns an error if
    /// the manifest could not be read or parsed.
    pub fn from_reader<R: Read>(reader: R) -> Result<PortalServerGlobalManifest> {
        let manifest: PortalServerGlobalManifest =
            from_reader(reader).context("failed to decode JSON global manifest")?;
        if manifest.format != 0 {
            return Err(anyhow!("unsupported manifest format {}", manifest.format));
        }
        Ok(manifest)
    }

    /// Returns the StoragePath for this portal server, returning the PHA bucket
    /// if is_pha is true, or the facilitator bucket otherwise.
    pub fn sum_part_bucket(&self, is_pha: bool) -> Result<StoragePath> {
        // For now, the path is assumed to be a GCS bucket.
        StoragePath::from_str(&format!(
            "gs://{}",
            if is_pha {
                &self.pha_sum_part_bucket
            } else {
                &self.facilitator_sum_part_bucket
            }
        ))
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

/// Obtains a manifest file from the provided URL
fn fetch_manifest(manifest_url: &str) -> Result<Response> {
    if !manifest_url.starts_with("https://") {
        return Err(anyhow!("Manifest must be fetched over HTTPS"));
    }
    let response = ureq::get(manifest_url)
        // By default, ureq will wait forever to connect or
        // read.
        .timeout_connect(10_000) // ten seconds
        .timeout_read(10_000) // ten seconds
        .call();
    if response.error() {
        return Err(anyhow!("failed to fetch manifest: {:?}", response));
    }
    Ok(response)
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
    if pem_key == "" {
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
    use std::io::Cursor;

    #[test]
    fn load_data_share_processor_global_manifest() {
        let reader = Cursor::new(
            r#"
{
    "format": 0,
    "server-identity": {
        "aws-account-id": 12345678901234567,
        "gcp-service-account-email": "service-account@project-name.iam.gserviceaccount.com"
    }
}
            "#,
        );
        let manifest = DataShareProcessorGlobalManifest::from_reader(reader).unwrap();
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
        let invalid_manifests = vec![
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
        ];

        for invalid_manifest in &invalid_manifests {
            let reader = Cursor::new(invalid_manifest);
            DataShareProcessorGlobalManifest::from_reader(reader).unwrap_err();
        }
    }

    #[test]
    fn load_specific_manifest() {
        let reader = Cursor::new(format!(
            r#"
{{
    "format": 0,
    "packet-encryption-certificates": {{
        "fake-key-1": {{
            "certificate": "who cares"
        }}
    }},
    "batch-signing-public-keys": {{
        "fake-key-2": {{
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n"
      }}
    }},
    "ingestion-bucket": "us-west-1/ingestion",
    "peer-validation-bucket": "us-west-1/validation"
}}
    "#,
            DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO
        ));
        let manifest = SpecificManifest::from_reader(reader).unwrap();

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
        let mut expected_packet_encryption_certificates = HashMap::new();
        expected_packet_encryption_certificates.insert(
            "fake-key-1".to_owned(),
            PacketEncryptionCertificate {
                certificate: "who cares".to_owned(),
            },
        );
        let expected_manifest = SpecificManifest {
            format: 0,
            batch_signing_public_keys: expected_batch_keys,
            packet_encryption_certificates: expected_packet_encryption_certificates,
            ingestion_bucket: "us-west-1/ingestion".to_string(),
            peer_validation_bucket: "us-west-1/validation".to_string(),
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

        if let StoragePath::S3Path(path) = manifest.validation_bucket().unwrap() {
            assert_eq!(
                path,
                S3Path {
                    region: Region::UsWest1,
                    bucket: "validation".to_owned(),
                    key: "".to_owned(),
                }
            );
        } else {
            assert!(false, "unexpected storage path type");
        }
    }

    #[test]
    fn invalid_specific_manifest() {
        let invalid_manifests = vec![
            "not-json",
            "{ \"missing\": \"keys\"}",
            // No format key
            r#"
{
    "packet-encryption-certificates": {
        "fake-key-1": {
            "certificate": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "peer-validation-bucket": "us-west-1/validation"
}
    "#,
            // Format key with wrong value
            r#"
{
    "format": 1,
    "packet-encryption-certificates": {
        "fake-key-1": {
            "certificate": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "peer-validation-bucket": "us-west-1/validation"
}
    "#,
            // Format key with wrong type
            r#"
{
    "format": "zero",
    "packet-encryption-certificates": {
        "fake-key-1": {
            "certificate": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "peer-validation-bucket": "us-west-1/validation"
}
    "#,
        ];

        for invalid_manifest in &invalid_manifests {
            let reader = Cursor::new(invalid_manifest);
            SpecificManifest::from_reader(reader).unwrap_err();
        }
    }

    #[test]
    fn invalid_specific_public_key() {
        let manifests_with_invalid_public_keys = vec![
            // Wrong PEM block
            r#"
{
    "format": 0,
    "packet-encryption-certificates": {
        "fake-key-1": {
            "certificate": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN EC PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEIKh3MccE1cdSF4pnEb+U0MmGYfkoQzOl2aiaJ6D9ZudqDdGiyA9YSUq3yia56nYJh5mk+HlzTX+AufoNR2bfrg==\n-----END EC PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "peer-validation-bucket": "us-west-1/validation"
}
    "#,
            // PEM contents not an ASN.1 SPKI
            r#"
{
    "format": 0,
    "packet-encryption-certificates": {
        "fake-key-1": {
            "certificate": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nBIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==\n-----END PUBLIC KEY-----"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "peer-validation-bucket": "us-west-1/validation"
}
    "#,
            // PEM contents too short
            r#"
{
    "format": 0,
    "packet-encryption-certificates": {
        "fake-key-1": {
            "certificate": "who cares"
        }
    },
    "batch-signing-public-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\ndG9vIHNob3J0Cg==\n-----END PUBLIC KEY-----\n"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "peer-validation-bucket": "us-west-1/validation"
}
    "#,
        ];
        for invalid_manifest in &manifests_with_invalid_public_keys {
            let reader = Cursor::new(invalid_manifest);
            let manifest = SpecificManifest::from_reader(reader).unwrap();
            assert!(manifest.batch_signing_public_keys().is_err());
        }
    }

    #[test]
    fn load_ingestor_global_manifest() {
        let manifest_with_aws_identity = r#"
{
    "format": 0,
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role"
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
    "format": 0,
    "server-identity": {
        "google-service-account": "112310747466759665351"
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
            IngestionServerGlobalManifest::from_reader(Cursor::new(manifest_with_aws_identity))
                .unwrap();
        assert_eq!(
            manifest.server_identity.aws_iam_entity,
            Some("arn:aws:iam::338276578713:role/ingestor-1-role".to_owned())
        );
        assert_eq!(manifest.server_identity.google_service_account, None);
        let batch_signing_public_keys = manifest.batch_signing_public_keys().unwrap();
        batch_signing_public_keys.get("key-identifier-1").unwrap();
        assert!(batch_signing_public_keys.get("nosuchkey").is_none());

        let manifest =
            IngestionServerGlobalManifest::from_reader(Cursor::new(manifest_with_gcp_identity))
                .unwrap();
        assert_eq!(manifest.server_identity.aws_iam_entity, None);
        assert_eq!(
            manifest.server_identity.google_service_account,
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
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role"
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
    "format": 1,
    "server-identity": {
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role"
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
        "aws-iam-entity": "arn:aws:iam::338276578713:role/ingestor-1-role"
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
            let reader = Cursor::new(invalid_manifest);
            IngestionServerGlobalManifest::from_reader(reader).unwrap_err();
        }
    }

    #[test]
    fn load_portal_global_manifest() {
        let manifest = r#"
{
    "format": 0,
    "facilitator-sum-part-bucket": "facilitator-bucket",
    "pha-sum-part-bucket": "pha-bucket"
}
            "#;

        let manifest = PortalServerGlobalManifest::from_reader(Cursor::new(manifest)).unwrap();
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
        if let StoragePath::GCSPath(path) = manifest.sum_part_bucket(true).unwrap() {
            assert_eq!(
                path,
                GCSPath {
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
    "facilitator-sum-part-bucket": "facilitator-bucket",
    "pha-sum-part-bucket": "pha-bucket"
}
    "#,
            // Format key with wrong value
            r#"
{
    "format": 1,
    "facilitator-sum-part-bucket": "facilitator-bucket",
    "pha-sum-part-bucket": "pha-bucket"
}
    "#,
            // Format key with wrong type
            r#"
{
    "format": "zero",
    "facilitator-sum-part-bucket": "facilitator-bucket",
    "pha-sum-part-bucket": "pha-bucket"
}
    "#,
            // Missing field
            r#"
{
    "format": 0,
    "facilitator-sum-part-bucket": "facilitator-bucket",
}
    "#,
        ];

        for invalid_manifest in &invalid_manifests {
            let reader = Cursor::new(invalid_manifest);
            PortalServerGlobalManifest::from_reader(reader).unwrap_err();
        }
    }
}
