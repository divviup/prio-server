use anyhow::{anyhow, Context, Result};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
use serde::Deserialize;
use serde_json::from_reader;
use std::{collections::HashMap, io::Read};

// See discussion in SpecificManifest::batch_signing_public_key
const ECDSA_P256_SPKI_PREFIX: &[u8] = &[
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

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

/// Represents a specific manifest, used to exchange configuration parameters
/// with peer data share processors. See the design document for the full
/// specification.
/// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
struct SpecificManifest {
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
    fn from_https(base_path: &str, peer_name: &str) -> Result<SpecificManifest> {
        if !base_path.starts_with("https://") {
            return Err(anyhow!("Manifest must be fetched over HTTPS"));
        }
        let manifest_url = format!("{}/{}/specific-manifest.json", base_path, peer_name);
        let response = ureq::get(&manifest_url)
            // By default, ureq will wait forever to connect or
            // read.
            .timeout_connect(10_000) // ten seconds
            .timeout_read(10_000) // ten seconds
            .call();
        if response.error() {
            return Err(anyhow!("failed to fetch specific manifest: {:?}", response));
        }
        SpecificManifest::from_reader(response.into_reader())
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
        // No Rust crate that we have found gives us an easy way to parse PKIX
        // SubjectPublicKeyInfo structures to get at the public key which can
        // then be used in ring::signature. Since we know the keys we deal with
        // should always be ECDSA P256, we can instead check that the binary
        // blob inside the PEM has the expected prefix for this kind of key in
        // this kind of encoding, as suggested in this GitHub issue on ring:
        // https://github.com/briansmith/ring/issues/881
        let key = self
            .batch_signing_public_keys
            .get(identifier)
            .context(format!("no value for key {}", identifier))?;

        let pem = pem::parse(&key.public_key)
            .context(format!("failed to parse key entry {} as PEM", identifier))?;
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
            &ECDSA_P256_SHA256_FIXED,
            Vec::from(key),
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
        "fake-key-1": {{
            "certificate": "who cares"
        }}
    }},
    "batch-signing-public-keys": {{
        "fake-key-2": {{
        "expiration": "",
        "public-key": "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----"
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
                    "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
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
    fn invalid_public_key() {
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
        "public-key": "-----BEGIN PUBLIC KEY-----\ndG9vIHNob3J0Cg==\n-----END PUBLIC KEY-----"
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
            assert!(manifest.batch_signing_public_key("fake-key-1").is_err());
        }
    }
}
