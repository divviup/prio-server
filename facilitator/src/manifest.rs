use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, Url};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
use serde::Deserialize;
use serde_json::from_reader;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

// See discussion in SpecificManifest::batch_signing_public_key
const ECDSA_P256_SPKI_PREFIX: &[u8] = &[
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// Represents the description of a batch signing key in a specific manifest.
#[derive(Debug, Deserialize, PartialEq)]
struct BatchSigningKey {
    /// The PEM-armored base64 encoding of the ASN.1 encoding of the PKIX
    /// SubjectPublicKeyInfo structure of an ECDSA P256 key.
    #[serde(rename = "public-key")]
    public_key: String,
    /// The ISO 8601 encoded UTC date at which this key expires.
    expiration: String,
}

/// Represents a specific manifest, used to exchange configuration parameters
/// with peer data share processors. See the design document for the full
/// specification.
/// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
#[derive(Debug, Deserialize, PartialEq)]
struct SpecificManifest {
    /// Format version of the manifest. Versions besides the currently supported
    /// one are rejected.
    format: u32,
    /// Region and name of the ingestion S3 bucket owned by this data share
    /// processor.
    #[serde(rename = "ingestion-bucket")]
    ingestion_bucket: String,
    /// Region and name of the validation S3 bucket owned by this data share
    /// processor.
    #[serde(rename = "validation-bucket")]
    validation_bucket: String,
    /// Region and name of the sum part S3 bucket owned by this data share
    /// processor.]
    #[serde(rename = "sum-part-bucket")]
    sum_part_bucket: String,
    /// Keys used by this data share processor to sign batches.
    #[serde(rename = "batch-signing-keys")]
    batch_signing_keys: HashMap<String, BatchSigningKey>,
    /// Keys used by this data share processor to decrypt ingestion packets. The
    /// values are a PEM encoded X.509 certificate containing the ECDSA P256
    /// public key.
    #[serde(rename = "packet-encryption-certificates")]
    packet_encryption_certificates: HashMap<String, String>,
}

impl SpecificManifest {
    fn from_https(base_path: &str, peer_name: &str) -> Result<SpecificManifest> {
        let base = Url::parse(base_path).context("failed to parse base path into URL")?;
        let mut manifest_url = base
            .join(peer_name)
            .context("failed to join URL component")?
            .join("specific-manifest.json")
            .context("failed to join URL component")?;
        manifest_url
            .set_scheme("https")
            .map_err(|_| anyhow!("failed to set URL scheme to HTTPS"))?;

        // reqwest::blocking::RequestBuilder::send() gives us a Response, which
        // implements std::io::Read by reading from the response body.
        // https://docs.rs/reqwest/0.10.8/src/reqwest/blocking/response.rs.html#397-405
        SpecificManifest::from_reader(
            Client::new()
                .get(manifest_url)
                .send()
                .context("failed to fetch specific manifest")?,
        )
    }

    fn from_file(path: &Path) -> Result<SpecificManifest> {
        SpecificManifest::from_reader(File::open(path).context("failed to open manifest file")?)
    }

    fn from_reader<R: Read>(reader: R) -> Result<SpecificManifest> {
        let manifest: SpecificManifest =
            from_reader(reader).context("failed to decode JSON specific manifest")?;
        if manifest.format != 0 {
            return Err(anyhow!("unsupported manifest format {}", manifest.format));
        }
        Ok(manifest)
    }

    /// Returns the ECDSA P256 public key corresponding to the provided key
    /// identifier, if it exists in the manifest.
    fn batch_signing_public_key(&self, identifier: &str) -> Result<UnparsedPublicKey<Vec<u8>>> {
        let key = self
            .batch_signing_keys
            .get(identifier)
            .context(format!("no value for key {}", identifier))?;

        let pem = pem::parse(&key.public_key)
            .context(format!("failed to parse key entry {} as PEM", identifier))?;
        if pem.tag != "PUBLIC KEY" {
            return Err(anyhow!(
                "key for identifier {} is not a PEM encoded public key"
            ));
        }
        if pem.contents.len() < ECDSA_P256_SPKI_PREFIX.len() {
            return Err(anyhow!("PEM contents not long enough to contain ASN.1 encoded ECDSA P256 SubjectPublicKeyInfo"));
        }
        if &pem.contents[..ECDSA_P256_SPKI_PREFIX.len()] != ECDSA_P256_SPKI_PREFIX {
            return Err(anyhow!(
                "PEM contents are not ASN.1 encoded ECDSA P256 SubjectPublicKeyInfo"
            ));
        }

        Ok(UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_FIXED,
            Vec::from(&pem.contents[ECDSA_P256_SPKI_PREFIX.len()..]),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{
        default_ingestor_private_key, DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO,
    };
    use ring::rand::SystemRandom;
    use std::io::Cursor;

    #[test]
    fn load_manifest() {
        let reader = Cursor::new(format!(
            r#"
{{
    "format": 0,
    "packet-encryption-certificates": {{
        "fake-key-1": "who cares"
    }},
    "batch-signing-keys": {{
        "fake-key-2": {{
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
        "purpose": "batch-signing"
      }}
    }},
    "ingestion-bucket": "us-west-1/ingestion",
    "sum-part-bucket": "us-west-1/sum-part",
    "validation-bucket": "us-west-1/validation"
}}
    "#,
            DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO
        ));
        let manifest = SpecificManifest::from_reader(reader).unwrap();

        let mut expected_batch_keys = HashMap::new();
        expected_batch_keys.insert(
            "fake-key-2".to_owned(),
            BatchSigningKey {
                expiration: "".to_string(),
                public_key: format!(
                    "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
                    DEFAULT_INGESTOR_SUBJECT_PUBLIC_KEY_INFO
                ),
            },
        );
        let mut expected_packet_encryption_certificates = HashMap::new();
        expected_packet_encryption_certificates
            .insert("fake-key-1".to_owned(), "who cares".to_owned());
        let expected_manifest = SpecificManifest {
            format: 0,
            batch_signing_keys: expected_batch_keys,
            packet_encryption_certificates: expected_packet_encryption_certificates,
            ingestion_bucket: "us-west-1/ingestion".to_string(),
            validation_bucket: "us-west-1/validation".to_string(),
            sum_part_bucket: "us-west-1/sum-part".to_string(),
        };
        assert_eq!(manifest, expected_manifest);
        let batch_signing_key = manifest.batch_signing_public_key("fake-key-2").unwrap();
        let content = b"some content";
        let signature = default_ingestor_private_key()
            .sign(&SystemRandom::new(), content)
            .unwrap();
        batch_signing_key
            .verify(content, signature.as_ref())
            .unwrap();
    }

    #[test]
    fn invalid_manifest() {
        let invalid_manifests = vec![
            "not-json",
            "{ \"missing\": \"keys\"}",
            // No format key
            r#"
{
    "packet-encryption-certificates": {
        "fake-key-1": "who cares"
    },
    "batch-signing-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----",
        "purpose": "batch-signing"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "sum-part-bucket": "us-west-1/sum-part",
    "validation-bucket": "us-west-1/validation"
}}
    "#,
            // Format key with wrong value
            r#"
{
    "format": 1,
    "packet-encryption-certificates": {
        "fake-key-1": "who cares"
    },
    "batch-signing-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----",
        "purpose": "batch-signing"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "sum-part-bucket": "us-west-1/sum-part",
    "validation-bucket": "us-west-1/validation"
}}
    "#,
            // Format key with wrong type
            r#"
{
    "format": "zero",
    "packet-encryption-certificates": {
        "fake-key-1": "who cares"
    },
    "batch-signing-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----",
        "purpose": "batch-signing"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "sum-part-bucket": "us-west-1/sum-part",
    "validation-bucket": "us-west-1/validation"
}}
    "#,
        ];

        for invalid_manifest in &invalid_manifests {
            let reader = Cursor::new(invalid_manifest);
            SpecificManifest::from_reader(reader).unwrap_err();
        }
    }

    #[test]
    fn invalid_public_key() {
        let manifests_with_invalid_public_keys = vec![
            // Wrong PEM block
            r#"
{
    "format": 0,
    "packet-encryption-certificates": {
        "fake-key-1": "who cares"
    },
    "batch-signing-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN EC PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEIKh3MccE1cdSF4pnEb+U0MmGYfkoQzOl2aiaJ6D9ZudqDdGiyA9YSUq3yia56nYJh5mk+HlzTX+AufoNR2bfrg==\n-----END EC PUBLIC KEY-----",
        "purpose": "batch-signing"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "sum-part-bucket": "us-west-1/sum-part",
    "validation-bucket": "us-west-1/validation"
}
    "#,
            // PEM contents not an ASN.1 SPKI
            r#"
{
    "format": 0,
    "packet-encryption-certificates": {
        "fake-key-1": "who cares"
    },
    "batch-signing-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\nBIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==\n-----END PUBLIC KEY-----",
        "purpose": "batch-signing"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "sum-part-bucket": "us-west-1/sum-part",
    "validation-bucket": "us-west-1/validation"
}
    "#,
            // PEM contents too short
            r#"
{
    "format": 0,
    "packet-encryption-certificates": {
        "fake-key-1": "who cares"
    },
    "batch-signing-keys": {
        "fake-key-2": {
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\ndG9vIHNob3J0Cg==\n-----END PUBLIC KEY-----",
        "purpose": "batch-signing"
      }
    },
    "ingestion-bucket": "us-west-1/ingestion",
    "sum-part-bucket": "us-west-1/sum-part",
    "validation-bucket": "us-west-1/validation"
}
    "#,
        ];
        for invalid_manifest in &manifests_with_invalid_public_keys {
            let reader = Cursor::new(invalid_manifest);
            let manifest = SpecificManifest::from_reader(reader).unwrap();
            assert!(manifest.batch_signing_public_key("fake-key-1").is_err());
        }
    }
}
