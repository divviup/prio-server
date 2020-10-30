package manifest

type DataShareGlobalManifest struct {
	Format         int                     `json:"format"`
	ServerIdentity DataShareServerIdentity `json:"server-identity"`
}

type DataShareServerIdentity struct {
	AwsAccountId           int64  `json:"aws-account-id"`
	GcpServiceAccountEmail string `json:"gcp-service-account-email"`
}

type DataShareSpecificManifest struct {
	Format                       int                                    `json:"format"`
	IngestionBucket              string                                 `json:"ingestion-bucket"`
	PeerValidationBucket         string                                 `json:"peer-validation-bucket"`
	BatchSigningPublicKeys       map[string]SigningKey                  `json:"batch-signing-public-keys"`
	PacketEncryptionCertificates map[string]PacketEncryptionCertificate `json:"packet-encryption-certificates"`
}

type SigningKey struct {
	PublicKey  string `json:"public-key"`
	Expiration string `json:"expiration"`
}

type PacketEncryptionCertificate struct {
	Certificate string `json:"certificate"`
}

type IngestionServerGlobalManifest struct {
	Format                 int                     `json:"format"`
	ServerIdentity         IngestionServerIdentity `json:"server-identity"`
	BatchSigningPublicKeys map[string]SigningKey   `json:"batch-signing-public-keys"`
}

type IngestionServerIdentity struct {
	AwsIamEntity         *string `json:"aws-iam-entity,omitempty"`
	GoogleServiceAccount *string `json:"google-service-account,omitempty"`
}
