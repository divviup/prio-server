package manifest

import (
	"fmt"
	"time"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

// DataShareProcessorSpecificManifest represents the manifest file advertised by
// a data share processor. See the design document for the full specification.
// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
type DataShareProcessorSpecificManifest struct {
	// Format is the version of the manifest.
	Format int64 `json:"format"`
	// IngestionIdentity is the identity of the ingestion and is only necessary
	// when an aws s3 ingestion server is used
	IngestionIdentity string `json:"ingestion-identity,omitempty"`
	// IngestionBucket is the region+name of the bucket that the data share
	// processor which owns the manifest reads ingestion batches from.
	IngestionBucket string `json:"ingestion-bucket"`
	// PeerValidationIdentity is the identity is the identity that should be
	// assumed by peers to write to the PeerValidationBucket
	PeerValidationIdentity string `json:"peer-validation-identity,omitempty"`
	// PeerValidationBucket is the region+name of the bucket that the data share
	// processor which owns the manifest reads peer validation batches from.
	PeerValidationBucket string `json:"peer-validation-bucket"`
	// BatchSigningPublicKeys maps key identifiers to batch signing public keys.
	// These are the keys that peers reading batches emitted by this data share
	// processor use to verify signatures.
	BatchSigningPublicKeys BatchSigningPublicKeys `json:"batch-signing-public-keys"`
	// PacketEncryptionKeyCSRs maps key identifiers to packet encryption CSRs.
	// The values are PEM encoded PKCS#10 self signed certificate signing
	// request, which contain the public key corresponding to the ECDSA P256
	// private key that the data share processor which owns the manifest uses to
	// decrypt ingestion share packets.
	PacketEncryptionKeyCSRs PacketEncryptionKeyCSRs `json:"packet-encryption-keys"`
}

// UpdateKeysConfig configures an UpdateKeys operation.
type UpdateKeysConfig struct {
	BatchSigningKey         key.Key // the key used for batch signing operations
	BatchSigningKeyIDPrefix string  // the key ID prefix to use for batch signing keys

	PacketEncryptionKey         key.Key // the key used for packet encryption operations
	PacketEncryptionKeyIDPrefix string  // the key ID prefix to use for packet encryption keys
	PacketEncryptionKeyCSRFQDN  string  // the FQDN to specify for packet encryption key CSRs
}

func (m DataShareProcessorSpecificManifest) UpdateKeys(cfg UpdateKeysConfig) (DataShareProcessorSpecificManifest, error) {
	// Copy the current manifest, clearing any existing batch signing/packet encryption keys.
	newM := m
	newM.BatchSigningPublicKeys, newM.PacketEncryptionKeyCSRs = BatchSigningPublicKeys{}, PacketEncryptionKeyCSRs{}

	// Update batch signing key.
	for _, v := range cfg.BatchSigningKey {
		kid := cfg.BatchSigningKeyIDPrefix
		if !v.CreationTime.IsZero() {
			kid = fmt.Sprintf("%s-%d", cfg.BatchSigningKeyIDPrefix, v.CreationTime.Unix())
		}

		var newBSPK BatchSigningPublicKey
		if bspk, ok := m.BatchSigningPublicKeys[kid]; ok {
			newBSPK = bspk
		} else {
			pkix, err := v.RawKey.PublicAsPKIX()
			if err != nil {
				return DataShareProcessorSpecificManifest{}, fmt.Errorf("couldn't create PKIX-encoding for batch signing key version created at %v (%d): %w", v.CreationTime.Format(time.RFC3339), v.CreationTime.Unix(), err)
			}
			newBSPK = BatchSigningPublicKey{PublicKey: pkix}
		}
		newM.BatchSigningPublicKeys[kid] = newBSPK
	}

	// Update packet encryption key.
	for _, v := range cfg.PacketEncryptionKey {
		if !v.Primary {
			continue
		}

		kid := cfg.PacketEncryptionKeyIDPrefix
		if !v.CreationTime.IsZero() {
			kid = fmt.Sprintf("%s-%d", cfg.PacketEncryptionKeyIDPrefix, v.CreationTime.Unix())
		}

		var newPEC PacketEncryptionCertificate
		if pec, ok := m.PacketEncryptionKeyCSRs[kid]; ok {
			newPEC = pec
		} else {
			csr, err := v.RawKey.PublicAsCSR(cfg.PacketEncryptionKeyCSRFQDN)
			if err != nil {
				return DataShareProcessorSpecificManifest{}, fmt.Errorf("couldn't create CSR for packet encryption key version created at %v (%d): %w", v.CreationTime.Format(time.RFC3339), v.CreationTime.Unix(), err)
			}
			newPEC = PacketEncryptionCertificate{CertificateSigningRequest: csr}
		}
		newM.PacketEncryptionKeyCSRs[kid] = newPEC
	}

	return newM, nil
}

// IngestorGlobalManifest represents the global manifest file for an ingestor.
type IngestorGlobalManifest struct {
	// Format is the version of the manifest.
	Format int64 `json:"format"`
	// ServerIdentity represents the server identity for the advertising party
	// of the manifest.
	ServerIdentity ServerIdentity `json:"server-identity"`
	// BatchSigningPublicKeys maps key identifiers to batch signing public keys.
	// These are the keys that will be used by the ingestion server advertising
	// this manifest to sign ingestion batches.
	BatchSigningPublicKeys BatchSigningPublicKeys `json:"batch-signing-public-keys"`
}

// ServerIdentity represents the server identity for the advertising party of
// the manifest.
type ServerIdentity struct {
	// AWSIamEntity is ARN of user or role - apple only
	AWSIamEntity string `json:"aws-iam-entity"`
	// GCPServiceAccountID is the numeric unique service account ID
	GCPServiceAccountID string `json:"gcp-service-account-id"`
	// GCPServiceAccountEmail is the email address of the gcp service account
	GCPServiceAccountEmail string `json:"gcp-service-account-email"`
}

type BatchSigningPublicKeys = map[string]BatchSigningPublicKey
type PacketEncryptionKeyCSRs = map[string]PacketEncryptionCertificate

// BatchSigningPublicKey represents a public key used for batch signing.
type BatchSigningPublicKey struct {
	// PublicKey is the PEM armored base64 encoding of the ASN.1 encoding of the
	// PKIX SubjectPublicKeyInfo structure. It must be an ECDSA P256 key.
	PublicKey string `json:"public-key"`
	// Expiration is the ISO 8601 encoded UTC date at which this key expires.
	Expiration string `json:"expiration,omitempty"`
}

// PacketEncryptionCertificate represents a certificate containing a public key
// used for packet encryption.
type PacketEncryptionCertificate struct {
	// CertificateSigningRequest is the PEM armored PKCS#10 CSR
	CertificateSigningRequest string `json:"certificate-signing-request"`
}
