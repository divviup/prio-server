package manifest

// DataShareProcessorSpecificManifest represents the manifest file advertised by a data share
// processor. See the design document for the full specification.
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
	// PeerValidationBucket is the region+name of the bucket that the data share
	// processor which owns the manifest reads peer validation batches from.
	PeerValidationBucket string `json:"peer-validation-bucket"`
	// BatchSigningPublicKeys maps key identifiers to batch signing public keys.
	// These are the keys that peers reading batches emitted by this data share
	// processor use to verify signatures.
	BatchSigningPublicKeys BatchSigningPublicKeys `json:"batch-signing-public-keys"`
	// PacketEncryptionKeyCSRs maps key identifiers to packet encryption
	// CSRs. The values are PEM encoded PKCS#10 self signed certificate signing request,
	// which contain the public key corresponding to the ECDSA P256 private key
	// that the data share processor which owns the manifest uses to decrypt
	// ingestion share packets.
	PacketEncryptionKeyCSRs PacketEncryptionKeyCSRs `json:"packet-encryption-keys"`
}

// IngestorGlobalManifest represents the global manifest file for an ingestor
type IngestorGlobalManifest struct {
	// Format is the version of the manifest.
	Format int64 `json:"format"`
	// ServerIdentity represents the server identity for the advertising party of the manifest
	ServerIdentity ServerIdentity `json:"server-identity"`
	// BatchSigningPublicKeys maps key identifiers to batch signing public keys.
	// These are the keys that will be used to sign batches coming from this service
	BatchSigningPublicKeys BatchSigningPublicKeys `json:"batch-signing-public-keys"`
}

// ServerIdentity represents the server identity for the advertising party of the manifest
type ServerIdentity struct {
	// AwsIamEntity is ARN of user or role - apple only
	AwsIamEntity string `json:"aws-iam-entity"`
	// GcpServiceAccountId is the numeric unique service account ID
	GcpServiceAccountId string `json:"gcp-service-account-id"`
	// GcpServiceAccountEmail is the email address of the gcp service account
	GcpServiceAccountEmail string `json:"gcp-service-account-email"`
}

type BatchSigningPublicKeys = map[string]BatchSigningPublicKey
type PacketEncryptionKeyCSRs = map[string]PacketEncryptionCertificate

// BatchSigningPublicKey represents a public key used for batch signing.
type BatchSigningPublicKey struct {
	// PublicKey is the PEM armored base64 encoding of the ASN.1 encoding of the
	// PKIX SubjectPublicKeyInfo structure. It must be an ECDSA P256 key.
	PublicKey string `json:"public-key"`
	// Expiration is the ISO 8601 encoded UTC date at which this key expires.
	Expiration string `json:"expiration"`
}

// PacketEncryptionCertificate represents a certificate containing a public key
// used for packet encryption.
type PacketEncryptionCertificate struct {
	// CertificateSigningRequest is the PEM armored PKCS#10 CSR
	CertificateSigningRequest string `json:"certificate-signing-request"`
}
