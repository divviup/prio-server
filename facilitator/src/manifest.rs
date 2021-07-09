use anyhow::{anyhow, Context, Result};
use elliptic_curve::sec1::{EncodedPoint, ToEncodedPoint};
use p256::{pkcs8::FromPublicKey, NistP256};
use pkix::{
    pem::{pem_to_der, PEM_CERTIFICATE_REQUEST},
    pkcs10::DerCertificationRequest,
    FromDer,
};
use prio::encrypt::{decrypt_share, encrypt_share, PrivateKey, PublicKey};
use ring::{
    rand::SystemRandom,
    signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1},
};
use serde::Deserialize;
use slog::Logger;
use std::collections::HashMap;

use crate::{
    config::{Identity, StoragePath},
    http, BatchSigningKey,
};

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

#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct PacketEncryptionCertificateSigningRequest {
    /// The PEM-armored base64 encoding of the ASN.1 encoding of a PKCS#10
    /// certificate signing request containing an ECDSA P256 key.
    certificate_signing_request: String,
}

impl PacketEncryptionCertificateSigningRequest {
    pub fn new(certificate_signing_request: String) -> Self {
        PacketEncryptionCertificateSigningRequest {
            certificate_signing_request,
        }
    }

    /// Gets the base64ed public key from the CSR that libprio-rs is expecting
    pub fn base64_public_key(&self) -> Result<String> {
        let der = pem_to_der(
            &self.certificate_signing_request,
            Some(PEM_CERTIFICATE_REQUEST),
        )
        .context("failed to parse pem for packet encryption certificate signing request")?;
        let csr = DerCertificationRequest::from_der(&der).context("failed to decode csr")?;

        let decoded_public_key: elliptic_curve::PublicKey<NistP256> =
            p256::PublicKey::from_public_key_der(&csr.reqinfo.spki.value)
                .map_err(|e| anyhow!("error when getting public key from der: {:?}", e))?;

        let encoded_point: EncodedPoint<NistP256> = decoded_public_key.to_encoded_point(false);

        let base64_public_key = base64::encode(encoded_point);

        Ok(base64_public_key)
    }
}

pub type PacketEncryptionCertificateSigningRequests =
    HashMap<String, PacketEncryptionCertificateSigningRequest>;

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
    pub fn from_https(base_path: &str, logger: &Logger) -> Result<Self> {
        let manifest_url = format!("{}/global-manifest.json", base_path);
        DataShareProcessorGlobalManifest::from_slice(
            fetch_manifest(&manifest_url, logger)?.as_bytes(),
        )
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
    ingestion_bucket: StoragePath,
    /// The ARN of the AWS IAM role that should be assumed by an ingestion
    /// server to write to this data share processor's ingestion bucket, if the
    /// ingestor does not have an AWS account of their own. This will not be
    /// present if the data share processor's ingestion bucket is not in AWS S3.
    ingestion_identity: Identity,
    /// URL of the validation bucket owned by this data share processor, which
    /// may be in the form "s3://{region}/{name}" or "gs://{name}".
    peer_validation_bucket: StoragePath,
    /// Keys used by this data share processor to sign batches.
    batch_signing_public_keys: HashMap<String, BatchSigningPublicKey>,
    /// Certificate signing requests containing public keys that should be used
    /// to encrypt ingestion share packets intended for this data share
    /// processor.
    packet_encryption_keys: PacketEncryptionCertificateSigningRequests,
}

impl SpecificManifest {
    /// Load the specific manifest for the specified peer relative to the
    /// provided base path. Returns an error if the manifest could not be
    /// downloaded or parsed.
    pub fn from_https(base_path: &str, peer_name: &str, logger: &Logger) -> Result<Self> {
        let manifest_url = format!("{}/{}-manifest.json", base_path, peer_name);
        SpecificManifest::from_slice(fetch_manifest(&manifest_url, logger)?.as_bytes())
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

    pub fn packet_decryption_keys(&self) -> Result<PacketEncryptionCertificateSigningRequests> {
        Ok(self.packet_encryption_keys.clone())
    }

    /// Returns the StoragePath for the data share processor's peer validation
    /// bucket.
    pub fn peer_validation_bucket(&self) -> &StoragePath {
        &self.peer_validation_bucket
    }

    /// Returns true if all the members of the parsed manifest are valid, false
    /// otherwise.
    pub fn validate(&self) -> Result<()> {
        self.batch_signing_public_keys()
            .context("bad manifest: public keys")?;
        Ok(())
    }

    /// Returns the identity that should be assumed to write to the data share
    /// processor's ingestion bucket
    pub fn ingestion_identity(&self) -> &Identity {
        &self.ingestion_identity
    }

    /// Returns the StoragePath for the data share processor's ingestion bucket
    pub fn ingestion_bucket(&self) -> &StoragePath {
        &self.ingestion_bucket
    }

    /// Checks if the batch signing public key in the manifest matches the
    /// provided batch signing private key by signing a random message and
    /// verifying the signature. Returns an error if the keys do not match.
    pub fn verify_batch_signing_key(
        &self,
        batch_signing_private_key: &BatchSigningKey,
    ) -> Result<()> {
        let test_message: Vec<u8> = (0..100).map(|_| rand::random::<u8>()).collect();
        let signature = batch_signing_private_key
            .key
            .sign(&SystemRandom::new(), &&test_message)
            .context(format!(
                "failed to sign test message with private key {}",
                batch_signing_private_key.identifier
            ))?;

        self.batch_signing_public_keys()?
            .get(&batch_signing_private_key.identifier)
            .context(format!(
                "key identifier {} not present in manifest batch signing public keys",
                batch_signing_private_key.identifier
            ))?
            .verify(&test_message, signature.as_ref())
            .context(format!(
                "failed to verify signature over test message with key {}",
                batch_signing_private_key.identifier
            ))
    }

    /// Checks if all of the packet encryption public keys in the manifest
    /// match one of the provided packet encryption private keys by encrypting a
    /// random message and decrypting it. Returns an error if any public key
    /// does not have a corresponding private key.
    pub fn verify_packet_encryption_keys(
        &self,
        packet_encryption_private_keys: &[PrivateKey],
    ) -> Result<()> {
        let test_message: Vec<u8> = (0..100).map(|_| rand::random::<u8>()).collect();
        'outer: for (identifier, csr) in &self.packet_encryption_keys {
            let public_key = PublicKey::from_base64(&csr.base64_public_key()?).context(format!(
                "failed to decode packet encryption public key {} from specific manifest",
                identifier,
            ))?;

            let encrypted = encrypt_share(&test_message, &public_key).context(format!(
                "failed to encrypt test message to packet encryption public key {}",
                identifier,
            ))?;

            for private_key in packet_encryption_private_keys {
                match decrypt_share(&encrypted, &private_key) {
                    Ok(decrypted) => {
                        // AEAD decryption succeeding but yielding incorrect
                        // plaintext is incredibly unlikely
                        if !decrypted.eq(&test_message) {
                            return Err(anyhow!(
                                "decrypted text does not match test message for key {}",
                                identifier
                            ));
                        }

                        // AEAD decryption succeeded and the plaintext is good,
                        // so move on to the next packet encryption public key
                        continue 'outer;
                    }
                    // AEAD decryption failing is expected if we are using the
                    // wrong private key, so move on to the next one
                    Err(_) => continue,
                }
            }

            // If we made it here, then no private key was able to decrypt the
            // test message and we have a key mismatch
            return Err(anyhow!(
                "unable to decrypt test message encrypted with {} with any of {} available packet decryption keys",
                identifier,
                packet_encryption_private_keys.len(),
            ));
        }

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
    pub fn from_https(base_path: &str, locality: Option<&str>, logger: &Logger) -> Result<Self> {
        IngestionServerManifest::from_http(base_path, locality, logger, fetch_manifest)
    }

    fn from_http(
        base_path: &str,
        locality: Option<&str>,
        logger: &Logger,
        fetcher: ManifestFetcher,
    ) -> Result<Self> {
        match fetcher(&format!("{}/global-manifest.json", base_path), logger) {
            Ok(body) => IngestionServerManifest::from_slice(body.as_bytes()),
            Err(err) => match locality {
                Some(locality) => IngestionServerManifest::from_slice(
                    fetcher(&format!("{}/{}-manifest.json", base_path, locality), logger)?
                        .as_bytes(),
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
    facilitator_sum_part_bucket: StoragePath,
    /// URL of the bucket to which PHA servers should write sum parts, which may
    /// be in the form "s3://{region}/{name}" or "gs://{name}".
    pha_sum_part_bucket: StoragePath,
}

impl PortalServerGlobalManifest {
    pub fn from_https(base_path: &str, logger: &Logger) -> Result<Self> {
        let manifest_url = format!("{}/global-manifest.json", base_path);
        PortalServerGlobalManifest::from_slice(fetch_manifest(&manifest_url, logger)?.as_bytes())
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
    pub fn sum_part_bucket(&self, is_pha: bool) -> &StoragePath {
        if is_pha {
            &self.pha_sum_part_bucket
        } else {
            &self.facilitator_sum_part_bucket
        }
    }
}

/// A function that fetches a manifest from the provided URL, returning the
/// manifest body as a String on success.
type ManifestFetcher = fn(&str, &Logger) -> Result<String>;

/// Obtains a manifest file from the provided URL, returning an error if the URL
/// is not https or if a problem occurs during the transfer.
fn fetch_manifest(manifest_url: &str, logger: &Logger) -> Result<String> {
    if !manifest_url.starts_with("https://") {
        return Err(anyhow!("Manifest must be fetched over HTTPS"));
    }
    http::simple_get_request(
        url::Url::parse(manifest_url)
            .context(format!("failed to parse manifest url: {}", manifest_url))?,
        logger,
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
    use crate::{
        config::{GcsPath, S3Path},
        logging::setup_test_logging,
        test_utils::{
            default_ingestor_private_key, default_packet_encryption_certificate_signing_request,
            DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO,
            DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST_PRIVATE_KEY,
            DEFAULT_PACKET_ENCRYPTION_CSR,
        },
    };
    use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
    use rusoto_core::Region;
    use std::{array::IntoIter, str::FromStr};
    use url::Url;

    fn url_fetcher(url: &str, logger: &Logger) -> Result<String> {
        http::simple_get_request(Url::parse(url)?, logger)
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
            "certificate-signing-request": "-----BEGIN CERTIFICATE REQUEST-----\n{}\n-----END CERTIFICATE REQUEST-----\n"
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
            DEFAULT_PACKET_ENCRYPTION_CSR, DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO
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
                certificate_signing_request: format!(
                    "-----BEGIN CERTIFICATE REQUEST-----\n{}\n-----END CERTIFICATE REQUEST-----\n",
                    DEFAULT_PACKET_ENCRYPTION_CSR
                ),
            },
        );
        let expected_manifest = SpecificManifest {
            format: 1,
            batch_signing_public_keys: expected_batch_keys,
            packet_encryption_keys: expected_packet_encryption_csrs,
            ingestion_bucket: StoragePath::from_str("s3://us-west-1/ingestion").unwrap(),
            ingestion_identity: Identity::from_str("arn:aws:iam:something:fake").unwrap(),
            peer_validation_bucket: StoragePath::from_str("gs://validation/path/fragment").unwrap(),
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

        assert_eq!(
            manifest.peer_validation_bucket(),
            &StoragePath::GcsPath(GcsPath {
                bucket: "validation".to_owned(),
                key: "path/fragment".to_owned(),
            }),
        );

        let packet_decryption_keys = manifest.packet_decryption_keys().unwrap();

        let packet_decryption_key = packet_decryption_keys.get("fake-key-1").unwrap();

        // Just checks that getting the base64'd public key doesn't error
        packet_decryption_key.base64_public_key().unwrap();

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

        assert_eq!(
            manifest.sum_part_bucket(false),
            &StoragePath::GcsPath(GcsPath {
                bucket: "facilitator-bucket".to_owned(),
                key: "".to_owned(),
            }),
        );
        assert_eq!(
            manifest.sum_part_bucket(true),
            &StoragePath::S3Path(S3Path {
                region: Region::UsWest1,
                bucket: "pha-bucket".to_owned(),
                key: "".to_owned(),
            }),
        );
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
    use prio::encrypt::{PrivateKey, PublicKey};

    #[test]
    fn ingestor_global_manifest() {
        let logger = setup_test_logging();
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

        IngestionServerManifest::from_http(&mockito::server_url(), None, &logger, url_fetcher)
            .unwrap();

        mocked_get.assert();
    }

    #[test]
    fn unparseable_ingestor_global_manifest() {
        let logger = setup_test_logging();
        let mocked_get = mock("GET", "/global-manifest.json")
            .with_status(200)
            .with_body("invalid manifest")
            .expect(1)
            .create();

        IngestionServerManifest::from_http(&mockito::server_url(), None, &logger, url_fetcher)
            .unwrap_err();

        mocked_get.assert();
    }

    #[test]
    fn ingestor_specific_manifest_fallback() {
        let logger = setup_test_logging();
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
            &logger,
            url_fetcher,
        )
        .unwrap();

        mocked_global_get.assert();
        mocked_specific_get.assert();
    }

    #[test]
    fn unparseable_ingestor_specific_manifest() {
        let logger = setup_test_logging();
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
            &logger,
            url_fetcher,
        )
        .unwrap_err();

        mocked_global_get.assert();
        mocked_specific_get.assert();
    }

    #[test]
    fn missing_ingestor_specific_manifest() {
        let logger = setup_test_logging();
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
            &logger,
            url_fetcher,
        )
        .unwrap_err();

        mocked_global_get.assert();
        mocked_specific_get.assert();
    }

    #[test]
    fn known_csr_and_private_key() {
        let csr = default_packet_encryption_certificate_signing_request();
        let private_key = PrivateKey::from_base64(
            DEFAULT_PACKET_ENCRYPTION_CERTIFICATE_SIGNING_REQUEST_PRIVATE_KEY,
        )
        .unwrap();

        let actual = PublicKey::from_base64(&csr.base64_public_key().unwrap()).unwrap();
        let expected = PublicKey::from(&private_key);

        // There's no convenient method of checking that two `PublicKey`s are
        // equal so we instead check that they both can encrypt a message to the
        // private key
        let test_message: Vec<u8> = (0..100).map(|_| rand::random::<u8>()).collect();

        let actual_encrypted = encrypt_share(&test_message, &actual).unwrap();
        let decrypted = decrypt_share(&actual_encrypted, &private_key).unwrap();
        assert_eq!(test_message, decrypted);

        let expected_encrypted = encrypt_share(&test_message, &expected).unwrap();
        let decrypted = decrypt_share(&&expected_encrypted, &private_key).unwrap();
        assert_eq!(test_message, decrypted);
    }

    #[test]
    fn test_key_consistency_checks() {
        // Real public and private keys ethically sourced from test environments
        let batch_signing_key_1_public =
            "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEf3\
            xFccRXkByhftlbQNjaOHG9qs+A\nOjF42iWxbMO8OH2vhT1c+ItsZ+gCzxg47aLpClG\
            dpgmI9fSh4R2WFhkuSA==\n-----END PUBLIC KEY-----\n";
        let batch_signing_key_1_private_pem =
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgQZztDnVQh43ty7pbDd\
            KpQd1bA+ZrV8gnoZs2nucwaaChRANCAAR/fEVxxFeQHKF+2VtA2No4cb2qz4A6MXjaJ\
            bFsw7w4fa+FPVz4i2xn6ALPGDjtoukKUZ2mCYj19KHhHZYWGS5I";
        let batch_signing_key_1_private = BatchSigningKey {
            key: EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING,
                &base64::decode(batch_signing_key_1_private_pem).unwrap(),
            )
            .unwrap(),
            identifier: "batch-signing-key-1".to_owned(),
        };

        let batch_signing_key_2_public =
            "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEkJ\
            jJIY7lxyO23EkcIROXkrASGVDy\nwkityouKiFbiahlIa/szIftNF3FoVAT+NnZWHBY\
            Cw3kSM4r2NEeGLAPNHA==\n-----END PUBLIC KEY-----\n";
        let batch_signing_key_2_private_pem =
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgP0q/CXcp9Dw6jwSRF0\
            u0a46WLs95dTFyv4hJnwsvrUmhRANCAASQmMkhjuXHI7bcSRwhE5eSsBIZUPLCSK3Ki\
            4qIVuJqGUhr+zMh+00XcWhUBP42dlYcFgLDeRIzivY0R4YsA80c";
        let batch_signing_key_2_private = BatchSigningKey {
            key: EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING,
                &base64::decode(batch_signing_key_2_private_pem).unwrap(),
            )
            .unwrap(),
            identifier: "batch-signing-key-2".to_owned(),
        };

        // Batch signing private key unrelated to any public key
        let batch_signing_key_unrelated_private_pem =
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgvFVl1jrTLnIUAVe5ER\
            VpPyZE6gBHSdu2fMExf78QsY2hRANCAAQoNHgIo7baQ8whCeor4k0abH2bGIMC9p6tE\
            Y4Wo/E/JIrWqbAFmYZOGK6Lq5pfrtwQG7Xaw2h3S6OtEk0fUmMW";
        let batch_signing_key_unrelated_private = BatchSigningKey {
            key: EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING,
                &base64::decode(batch_signing_key_unrelated_private_pem).unwrap(),
            )
            .unwrap(),
            identifier: "batch-signing-key-unrelated".to_owned(),
        };

        let packet_encryption_key_1_csr = "-----BEGIN CERTIFICATE REQUEST-----\n\
        MIHzMIGbAgEAMDkxNzA1BgNVBAMTLm5hcm5pYS50aW1nLWRldi1waGEuY2VydGlm\n\
        aWNhdGVzLmlzcmctcHJpby5vcmcwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAQZ\n\
        PaZ+nwLgkKyENDgKN8ygde4L5+YCXD2jF4YlYteiWG16jJyekqdswO2l3NB4mkYV\n\
        gGaMeyvlf2uYhX5CfPpFoAAwCgYIKoZIzj0EAwIDRwAwRAIgXZCNgsPQhHuKzyqZ\n\
        Gd/WhAaaAAXcBChLfXzlgqglxZ0CIDzHxmURqDLNDivx5x4M342aRG3izy11d0HS\n\
        USO9+luc\n\
        -----END CERTIFICATE REQUEST-----\n";
        let packet_encryption_key_1_private_b64 =
            "BBk9pn6fAuCQrIQ0OAo3zKB17gvn5gJcPaMXhiVi16JYbXqMnJ6Sp2zA7aXc0HiaRh\
            WAZox7K+V/a5iFfkJ8+kUqOtcCXXhjIRTS2XxFanmcfd+zTHNtjoNl1+a/N6q4/Q==";
        let packet_encryption_key_1_private =
            PrivateKey::from_base64(packet_encryption_key_1_private_b64).unwrap();

        let packet_encryption_key_2_csr = "-----BEGIN CERTIFICATE REQUEST-----\n\
        MIH0MIGbAgEAMDkxNzA1BgNVBAMTLmdvbmRvci50aW1nLWRldi1waGEuY2VydGlm\n\
        aWNhdGVzLmlzcmctcHJpby5vcmcwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAQk\n\
        L1UbdRTE3RqzM08NIsGxho4EcxXJ+GmpBkRgG0ogeIn4bALJnyh4FkrsIn162zqu\n\
        ajnEl5uxGopAturyO3ijoAAwCgYIKoZIzj0EAwIDSAAwRQIhAJfvxfn35cQWD5lU\n\
        6/EfiNwuY6LJg8UJC5WCb/MOq219AiBDHdKOyI/eo1+OZABR132+zZZAhwG9lA2V\n\
        eUcnSpurBw==\n\
        -----END CERTIFICATE REQUEST-----\n";
        let packet_encryption_key_2_private_b64 =
            "BCQvVRt1FMTdGrMzTw0iwbGGjgRzFcn4aakGRGAbSiB4ifhsAsmfKHgWSuwifXrbOq\
            5qOcSXm7EaikC26vI7eKMV3owPJiAmeCdfNVeO82olh8p4nASdSs3SssjZJVz2pw==";
        let packet_encryption_key_2_private =
            PrivateKey::from_base64(packet_encryption_key_2_private_b64).unwrap();

        let packet_encryption_key_unrelated_private_b64 =
            "BFyuN8wm2j+Dj7uM28vp/rmeA3badNALXDgBCLCO4x197sumOYr8doModklTjHbQSv\
            3vtOs7mixH6SlcbF7mZ2TeI4teI/nxinZPLXQSilVFA45fSJg3XTllJ4ic8ibYug==";
        let packet_encryption_key_unrelated_private =
            PrivateKey::from_base64(packet_encryption_key_unrelated_private_b64).unwrap();

        let specific_manifest = SpecificManifest {
            format: 1,
            ingestion_bucket: StoragePath::from_str("gs://irrelevant").unwrap(),
            ingestion_identity: Identity::none(),
            peer_validation_bucket: StoragePath::from_str("gs://irrelevant").unwrap(),
            batch_signing_public_keys: IntoIter::new([
                (
                    "batch-signing-key-1".to_owned(),
                    BatchSigningPublicKey {
                        public_key: batch_signing_key_1_public.to_owned(),
                        expiration: "irrelevant".to_owned(),
                    },
                ),
                (
                    "batch-signing-key-2".to_owned(),
                    BatchSigningPublicKey {
                        public_key: batch_signing_key_2_public.to_owned(),
                        expiration: "irrelevant".to_owned(),
                    },
                ),
            ])
            .collect(),
            packet_encryption_keys: IntoIter::new([
                (
                    "packet-encryption-key-1".to_owned(),
                    PacketEncryptionCertificateSigningRequest {
                        certificate_signing_request: packet_encryption_key_1_csr.to_owned(),
                    },
                ),
                (
                    "packet-encryption-key-2".to_owned(),
                    PacketEncryptionCertificateSigningRequest {
                        certificate_signing_request: packet_encryption_key_2_csr.to_owned(),
                    },
                ),
            ])
            .collect(),
        };

        // Passes because manifest has corresponding public key
        specific_manifest
            .verify_batch_signing_key(&batch_signing_key_1_private)
            .unwrap();
        // Passes because manifest has corresponding public key
        specific_manifest
            .verify_batch_signing_key(&batch_signing_key_2_private)
            .unwrap();
        // Fails because manifest does not contain corresponding public key
        specific_manifest
            .verify_batch_signing_key(&batch_signing_key_unrelated_private)
            .unwrap_err();

        // Passes because manifest contains both corresponding public keys
        specific_manifest
            .verify_packet_encryption_keys(&[
                packet_encryption_key_1_private.clone(),
                packet_encryption_key_2_private.clone(),
            ])
            .unwrap();
        // Passes because manifest contains both corresponding public keys;
        // extra private key is benign
        specific_manifest
            .verify_packet_encryption_keys(&[
                packet_encryption_key_1_private.clone(),
                packet_encryption_key_2_private.clone(),
                packet_encryption_key_unrelated_private.clone(),
            ])
            .unwrap();
        // Fails because one of the private keys corresponding to the manifest's
        // public keys is missing
        specific_manifest
            .verify_packet_encryption_keys(&[packet_encryption_key_1_private])
            .unwrap_err();
        // Fails because one of the private keys corresponding to the manifest's
        // public keys is missing
        specific_manifest
            .verify_packet_encryption_keys(&[packet_encryption_key_2_private])
            .unwrap_err();
        // Fails because none of the private keys corresponding to the manifest
        // public keys
        specific_manifest
            .verify_packet_encryption_keys(&[packet_encryption_key_unrelated_private])
            .unwrap_err();
    }
}
