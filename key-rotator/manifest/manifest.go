package manifest

import (
	"crypto/ecdsa"
	"crypto/x509"
	"encoding/pem"
	"errors"
	"fmt"
	"strings"
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

func (m DataShareProcessorSpecificManifest) equalModuloKeys(o DataShareProcessorSpecificManifest) bool {
	return m.Format == o.Format &&
		m.IngestionIdentity == o.IngestionIdentity &&
		m.IngestionBucket == o.IngestionBucket &&
		m.PeerValidationIdentity == o.PeerValidationIdentity &&
		m.PeerValidationBucket == o.PeerValidationBucket
}

// Equal returns true if and only if this manifest is equal to the given
// manifest.
func (m DataShareProcessorSpecificManifest) Equal(o DataShareProcessorSpecificManifest) bool {
	return m.equalModuloKeys(o) &&
		m.BatchSigningPublicKeys.Equal(o.BatchSigningPublicKeys) &&
		m.PacketEncryptionKeyCSRs.Equal(o.PacketEncryptionKeyCSRs)
}

// Diff returns a human-readable string describing the differences from the
// given `o` to this manifest, suitable for logging. Diff returns the empty
// string if and only if the two keys are equal.
func (m DataShareProcessorSpecificManifest) Diff(o DataShareProcessorSpecificManifest) string {
	// Build up structures allowing easy generation of diffs.
	bskInfos := map[string]struct{ old, new *BatchSigningPublicKey }{}
	for kid, key := range m.BatchSigningPublicKeys {
		key := key
		info := bskInfos[kid]
		info.new = &key
		bskInfos[kid] = info
	}
	for kid, key := range o.BatchSigningPublicKeys {
		key := key
		info := bskInfos[kid]
		info.old = &key
		bskInfos[kid] = info
	}

	pekInfos := map[string]struct{ old, new *PacketEncryptionCertificate }{}
	for kid, key := range m.PacketEncryptionKeyCSRs {
		key := key
		info := pekInfos[kid]
		info.new = &key
		pekInfos[kid] = info
	}
	for kid, key := range o.PacketEncryptionKeyCSRs {
		key := key
		info := pekInfos[kid]
		info.old = &key
		pekInfos[kid] = info
	}

	// Generate diffs.
	var diffs []string
	if m.Format != o.Format {
		diffs = append(diffs, fmt.Sprintf("changed format %d → %d", o.Format, m.Format))
	}
	if m.IngestionIdentity != o.IngestionIdentity {
		diffs = append(diffs, fmt.Sprintf("changed ingestion identity %q → %q", o.IngestionIdentity, m.IngestionIdentity))
	}
	if m.IngestionBucket != o.IngestionBucket {
		diffs = append(diffs, fmt.Sprintf("changed ingestion bucket %q → %q", o.IngestionBucket, m.IngestionBucket))
	}
	if m.PeerValidationIdentity != o.PeerValidationIdentity {
		diffs = append(diffs, fmt.Sprintf("changed peer validation identity %q → %q", o.PeerValidationIdentity, m.PeerValidationIdentity))
	}
	if m.PeerValidationBucket != o.PeerValidationBucket {
		diffs = append(diffs, fmt.Sprintf("changed peer validation bucket %q → %q", o.PeerValidationBucket, m.PeerValidationBucket))
	}

	for kid, info := range bskInfos {
		switch {
		case info.old == nil:
			diffs = append(diffs, fmt.Sprintf("added batch signing key version %q", kid))
		case info.new == nil:
			diffs = append(diffs, fmt.Sprintf("removed batch signing key version %q", kid))
		case (*info.old) != (*info.new):
			diffs = append(diffs, fmt.Sprintf("modified key material for batch signing key version %q", kid))
		}
	}
	for kid, info := range pekInfos {
		switch {
		case info.old == nil:
			diffs = append(diffs, fmt.Sprintf("added packet encryption key version %q", kid))
		case info.new == nil:
			diffs = append(diffs, fmt.Sprintf("removed packet encryption key version %q", kid))
		case (*info.old) != (*info.new):
			diffs = append(diffs, fmt.Sprintf("modified key material for packet encryption key version %q", kid))
		}
	}

	return strings.Join(diffs, "; ")
}

// UpdateKeysConfig configures an UpdateKeys operation.
type UpdateKeysConfig struct {
	BatchSigningKey         key.Key // the key used for batch signing operations
	BatchSigningKeyIDPrefix string  // the key ID prefix to use for batch signing keys

	PacketEncryptionKey         key.Key // the key used for packet encryption operations
	PacketEncryptionKeyIDPrefix string  // the key ID prefix to use for packet encryption keys
	PacketEncryptionKeyCSRFQDN  string  // the FQDN to specify for packet encryption key CSRs

	SkipPreUpdateValidations  bool // if set, do not perform pre-update validation checks
	SkipPostUpdateValidations bool // if set, do not perform post-update validation checks
}

func (cfg UpdateKeysConfig) Validate() error {
	if cfg.BatchSigningKey.IsEmpty() {
		return errors.New("batch signing key has no key versions")
	}
	if cfg.PacketEncryptionKey.IsEmpty() {
		return errors.New("packet encryption key has no key versions")
	}
	return nil
}

func (cfg UpdateKeysConfig) batchSigningKeyID(ts int64) string {
	if ts != 0 {
		return fmt.Sprintf("%s-%d", cfg.BatchSigningKeyIDPrefix, ts)
	}
	return cfg.BatchSigningKeyIDPrefix
}

func (cfg UpdateKeysConfig) packetEncryptionKeyID(ts int64) string {
	if ts != 0 {
		return fmt.Sprintf("%s-%d", cfg.PacketEncryptionKeyIDPrefix, ts)
	}
	return cfg.PacketEncryptionKeyIDPrefix
}

func (m DataShareProcessorSpecificManifest) UpdateKeys(cfg UpdateKeysConfig) (DataShareProcessorSpecificManifest, error) {
	// Validate parameters.
	if err := cfg.Validate(); err != nil {
		return DataShareProcessorSpecificManifest{}, fmt.Errorf("invalid update config: %w", err)
	}
	if !cfg.SkipPreUpdateValidations {
		if err := validatePreUpdateManifest(cfg, m); err != nil {
			return DataShareProcessorSpecificManifest{}, fmt.Errorf("manifest pre-update validation error: %w", err)
		}
		if err := validateKeyMaterialAgainstManifest(cfg, m); err != nil {
			return DataShareProcessorSpecificManifest{}, fmt.Errorf("manifest pre-update validation error: %w", err)
		}
	}

	// Copy the current manifest, clearing any existing batch signing/packet encryption keys.
	newM := m
	newM.BatchSigningPublicKeys, newM.PacketEncryptionKeyCSRs = BatchSigningPublicKeys{}, PacketEncryptionKeyCSRs{}

	// Update batch signing key.
	if err := cfg.BatchSigningKey.Versions(func(v key.Version) error {
		kid := cfg.batchSigningKeyID(v.CreationTimestamp)
		var newBSPK BatchSigningPublicKey
		if bspk, ok := m.BatchSigningPublicKeys[kid]; ok {
			newBSPK = bspk
		} else {
			pkix, err := v.KeyMaterial.PublicAsPKIX()
			if err != nil {
				return fmt.Errorf("couldn't create PKIX-encoding for batch signing key version with creation timestamp %d: %w", v.CreationTimestamp, err)
			}
			const batchSigningPublicKeyValidityPeriod = 100 * 365 * 24 * time.Hour // 100 years
			newBSPK = BatchSigningPublicKey{
				PublicKey:  pkix,
				Expiration: time.Now().UTC().Add(batchSigningPublicKeyValidityPeriod).Format(time.RFC3339),
			}
		}
		newM.BatchSigningPublicKeys[kid] = newBSPK
		return nil
	}); err != nil {
		return DataShareProcessorSpecificManifest{}, err
	}

	// Update packet encryption key.
	primaryPEKVersion := cfg.PacketEncryptionKey.Primary()
	kid := cfg.packetEncryptionKeyID(primaryPEKVersion.CreationTimestamp)
	var newPEC PacketEncryptionCertificate
	if pec, ok := m.PacketEncryptionKeyCSRs[kid]; ok {
		newPEC = pec
	} else {
		csr, err := primaryPEKVersion.KeyMaterial.PublicAsCSR(cfg.PacketEncryptionKeyCSRFQDN)
		if err != nil {
			return DataShareProcessorSpecificManifest{}, fmt.Errorf("couldn't create CSR for packet encryption key version with creation timestamp %d: %w", primaryPEKVersion.CreationTimestamp, err)
		}
		newPEC = PacketEncryptionCertificate{CertificateSigningRequest: csr}
	}
	newM.PacketEncryptionKeyCSRs[kid] = newPEC

	// Validate results.
	if !cfg.SkipPostUpdateValidations {
		if err := validatePostUpdateManifest(cfg, newM, m); err != nil {
			return DataShareProcessorSpecificManifest{}, fmt.Errorf("manifest post-update validation error: %w", err)
		}
		if err := validateKeyMaterialAgainstManifest(cfg, newM); err != nil {
			return DataShareProcessorSpecificManifest{}, fmt.Errorf("manifest post-update validation error: %w", err)
		}
	}
	return newM, nil
}

func validatePreUpdateManifest(cfg UpdateKeysConfig, m DataShareProcessorSpecificManifest) error {
	// Pre-update, if the manifest includes any batch signing key versions, the
	// update config's batch signing key's primary version is already included
	// in the manifest.
	if len(m.BatchSigningPublicKeys) > 0 {
		kid := cfg.batchSigningKeyID(cfg.BatchSigningKey.Primary().CreationTimestamp)
		if _, ok := m.BatchSigningPublicKeys[kid]; !ok {
			return fmt.Errorf("update's batch signing key primary version %q not included in manifest", kid)
		}
	}

	// Pre-update, if the manifest includes any packet encryption key versions,
	// they are included in the update config's packet encryption key.
	if len(m.PacketEncryptionKeyCSRs) > 0 {
		pekKIDs := map[string]struct{}{}
		for kid := range m.PacketEncryptionKeyCSRs {
			pekKIDs[kid] = struct{}{}
		}
		_ = cfg.PacketEncryptionKey.Versions(func(v key.Version) error {
			kid := cfg.packetEncryptionKeyID(v.CreationTimestamp)
			delete(pekKIDs, kid)
			return nil
		})
		for kid := range pekKIDs {
			return fmt.Errorf("manifest packet encryption key version %q not included in update", kid)
		}
	}

	return nil
}

func validatePostUpdateManifest(cfg UpdateKeysConfig, m, oldM DataShareProcessorSpecificManifest) error {
	// Post-update, manifests must have at least one batch signing key version.
	if len(m.BatchSigningPublicKeys) == 0 {
		return errors.New("no batch signing public keys")
	}

	// Post-update, the key versions in the manifest's batch signing key must
	// match the key versions in the update config's batch signing key.
	kids := map[string]struct{}{}
	_ = cfg.BatchSigningKey.Versions(func(v key.Version) error {
		kid := cfg.batchSigningKeyID(v.CreationTimestamp)
		kids[kid] = struct{}{}
		return nil
	})
	for kid := range m.BatchSigningPublicKeys {
		if _, ok := kids[kid]; !ok {
			return fmt.Errorf("manifest included unexpected batch signing key version %q", kid)
		}
		delete(kids, kid)
	}
	for kid := range kids {
		return fmt.Errorf("manifest missing expected batch signing key version %q", kid)
	}

	// Post-update, manifests must have exactly one packet encryption key version.
	if len(m.PacketEncryptionKeyCSRs) != 1 {
		return fmt.Errorf("expected exactly one packet encryption public key (had %d)", len(m.PacketEncryptionKeyCSRs))
	}

	// Post-update, the sole version in the manifest's packet encryption key
	// must be the primary version in the update config.
	foundPEK := false
	pekKID := cfg.packetEncryptionKeyID(cfg.PacketEncryptionKey.Primary().CreationTimestamp)
	for kid := range m.PacketEncryptionKeyCSRs {
		if kid != pekKID {
			return fmt.Errorf("manifest included unexpected packet encryption key version %q", kid)
		}
		foundPEK = true
	}
	if !foundPEK {
		return fmt.Errorf("manifest missing expected packet encryption key version %q", pekKID)
	}

	// Post-update, manifests' non-key data must match pre-update manifest data exactly.
	if !m.equalModuloKeys(oldM) {
		return fmt.Errorf("non-key data modified")
	}

	// Post-update, manifests' key data for key versions that exist both pre- &
	// post-update must match exactly.
	for kid, key := range m.BatchSigningPublicKeys {
		if oldKey, ok := oldM.BatchSigningPublicKeys[kid]; ok {
			if key != oldKey {
				return fmt.Errorf("pre-existing batch signing key %q modified", kid)
			}
		}
	}
	for kid, key := range m.PacketEncryptionKeyCSRs {
		if oldKey, ok := oldM.PacketEncryptionKeyCSRs[kid]; ok {
			if key != oldKey {
				return fmt.Errorf("pre-existing packet encryption key %q modified", kid)
			}
		}
	}

	return nil
}

// validateKeyMaterialAgainstManifest verifies that, for any key versions that
// exist in both the update config's keys & the manifest's keys, the key
// material matches. No verification is done for key material that exists in
// only the update config's keys or only the manifest's keys.
func validateKeyMaterialAgainstManifest(cfg UpdateKeysConfig, m DataShareProcessorSpecificManifest) error {
	// Verify batch signing keys.
	if err := cfg.BatchSigningKey.Versions(func(v key.Version) error {
		kid := cfg.batchSigningKeyID(v.CreationTimestamp)
		bsk, ok := m.BatchSigningPublicKeys[kid]
		if !ok {
			return nil // key version does not exist in manifest
		}
		manifestPubkey, err := bsk.toPublicKey()
		if err != nil {
			return fmt.Errorf("couldn't parse batch signing key version %q from manifest: %w", kid, err)
		}
		if !manifestPubkey.Equal(v.KeyMaterial.Public()) {
			return fmt.Errorf("public key mismatch in batch signing key version %q", kid)
		}
		return nil
	}); err != nil {
		return err
	}

	// Verify packet encryption keys.
	if err := cfg.PacketEncryptionKey.Versions(func(v key.Version) error {
		kid := cfg.packetEncryptionKeyID(v.CreationTimestamp)
		pek, ok := m.PacketEncryptionKeyCSRs[kid]
		if !ok {
			return nil // key version does not exist in manifest
		}
		manifestPubkey, err := pek.toPublicKey()
		if err != nil {
			return fmt.Errorf("couldn't parse packet encryption key version %q from manifest: %w", kid, err)
		}
		if !manifestPubkey.Equal(v.KeyMaterial.Public()) {
			return fmt.Errorf("public key mismatch in packet encryption key version %q", kid)
		}
		return nil
	}); err != nil {
		return err
	}

	return nil
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

type BatchSigningPublicKeys map[string]BatchSigningPublicKey

func (b BatchSigningPublicKeys) Equal(o BatchSigningPublicKeys) bool {
	if len(b) != len(o) {
		return false
	}
	for k, bv := range b {
		ov, ok := o[k]
		if !ok || bv != ov {
			return false
		}
	}
	return true
}

type PacketEncryptionKeyCSRs map[string]PacketEncryptionCertificate

func (p PacketEncryptionKeyCSRs) Equal(o PacketEncryptionKeyCSRs) bool {
	if len(p) != len(o) {
		return false
	}
	for k, pv := range p {
		ov, ok := o[k]
		if !ok || pv != ov {
			return false
		}
	}
	return true
}

// BatchSigningPublicKey represents a public key used for batch signing.
type BatchSigningPublicKey struct {
	// PublicKey is the PEM armored base64 encoding of the ASN.1 encoding of the
	// PKIX SubjectPublicKeyInfo structure. It must be an ECDSA P256 key.
	PublicKey string `json:"public-key"`
	// Expiration is the ISO 8601 encoded UTC date at which this key expires.
	Expiration string `json:"expiration"`
}

func (k BatchSigningPublicKey) toPublicKey() (*ecdsa.PublicKey, error) {
	pemPKIX, _ := pem.Decode([]byte(k.PublicKey))
	if pemPKIX == nil {
		return nil, errors.New("couldn't parse as PEM")
	}
	pkix, err := x509.ParsePKIXPublicKey(pemPKIX.Bytes)
	if err != nil {
		return nil, fmt.Errorf("couldn't parse as PKIX: %w", err)
	}
	pub, ok := pkix.(*ecdsa.PublicKey)
	if !ok {
		return nil, fmt.Errorf("PKIX public key was a %T, want %T", pub, (*ecdsa.PublicKey)(nil))
	}
	return pub, nil
}

// PacketEncryptionCertificate represents a certificate containing a public key
// used for packet encryption.
type PacketEncryptionCertificate struct {
	// CertificateSigningRequest is the PEM armored PKCS#10 CSR
	CertificateSigningRequest string `json:"certificate-signing-request"`
}

func (k PacketEncryptionCertificate) toPublicKey() (*ecdsa.PublicKey, error) {
	pemCSR, _ := pem.Decode([]byte(k.CertificateSigningRequest))
	if pemCSR == nil {
		return nil, fmt.Errorf("couldn't parse as PEM")
	}
	csr, err := x509.ParseCertificateRequest(pemCSR.Bytes)
	if err != nil {
		return nil, fmt.Errorf("couldn't parse as CSR: %w", err)
	}
	pub, ok := csr.PublicKey.(*ecdsa.PublicKey)
	if !ok {
		return nil, fmt.Errorf("CSR public key was a %T, want %T", pub, (*ecdsa.PublicKey)(nil))
	}
	return pub, nil
}
