package manifest

import (
	"crypto/ecdsa"
	"crypto/x509"
	"encoding/pem"
	"fmt"
	"strings"
	"testing"

	"github.com/abetterinternet/prio-server/key-rotator/key"
	"github.com/google/go-cmp/cmp"
)

func TestUpdateKeys(t *testing.T) {
	t.Parallel()

	const (
		bskPrefix = "bsk"
		pekPrefix = "pek"
		fqdn      = "update.fqdn"
	)

	// Make up a bunch of distinct keys.
	k0, k1, k2, k3, k4, k5 := mustP256(), mustP256(), mustP256(), mustP256(), mustP256(), mustP256()

	// Success tests.
	for _, test := range []struct {
		name string

		// Initial manifest parameters.
		initialBSKs map[string]key.Material
		initialPEKs map[string]key.Material

		// UpdateKeys parameters.
		batchSigningKey     key.Key
		packetEncryptionKey key.Key

		// Desired output manifest parameters.
		wantBSKs map[string]key.Material
		wantPEKs map[string]key.Material
	}{
		{
			name:                "no keys at start (new environment rollout)",
			batchSigningKey:     k(kv(15, k1), kv(10, k0), kv(20, k2)),
			packetEncryptionKey: k(kv(20, k5), kv(10, k3), kv(15, k4)),
			wantBSKs: map[string]key.Material{
				"bsk-10": k0,
				"bsk-15": k1,
				"bsk-20": k2,
			},
			wantPEKs: map[string]key.Material{
				"pek-20": k5,
			},
		},
		{
			// we purposefully use different keys at same timestamp to test
			// that we keep old manifest data if the key IDs match up.
			name:                "keys already populated, old key material kept",
			initialBSKs:         map[string]key.Material{"bsk-10": k0},
			initialPEKs:         map[string]key.Material{"pek-10": k1},
			batchSigningKey:     k(kv(10, k2)),
			packetEncryptionKey: k(kv(10, k3)),
			wantBSKs:            map[string]key.Material{"bsk-10": k0},
			wantPEKs:            map[string]key.Material{"pek-10": k1},
		},

		// the following tests verify how manifest updates behave against a
		// simulated rotation of keys
		{
			name:                "before first rotation (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": k0},
			initialPEKs:         map[string]key.Material{"pek": k0},
			batchSigningKey:     k(kv(0, k0)),
			packetEncryptionKey: k(kv(0, k0)),
			wantBSKs:            map[string]key.Material{"bsk": k0},
			wantPEKs:            map[string]key.Material{"pek": k0},
		},
		{
			name:                "first new key (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": k0},
			initialPEKs:         map[string]key.Material{"pek": k0},
			batchSigningKey:     k(kv(0, k0), kv(10, k1)),
			packetEncryptionKey: k(kv(0, k0), kv(10, k1)),
			wantBSKs:            map[string]key.Material{"bsk": k0, "bsk-10": k1},
			wantPEKs:            map[string]key.Material{"pek": k0},
		},
		{
			name:                "first primary-key change (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": k0, "bsk-10": k1},
			initialPEKs:         map[string]key.Material{"pek": k0},
			batchSigningKey:     k(kv(10, k1), kv(0, k0)),
			packetEncryptionKey: k(kv(10, k1), kv(0, k0)),
			wantBSKs:            map[string]key.Material{"bsk": k0, "bsk-10": k1},
			wantPEKs:            map[string]key.Material{"pek-10": k1},
		},
		{
			name:                "first key removal (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": k0, "bsk-10": k1},
			initialPEKs:         map[string]key.Material{"pek-10": k1},
			packetEncryptionKey: k(kv(10, k1)),
			batchSigningKey:     k(kv(10, k1)),
			wantBSKs:            map[string]key.Material{"bsk-10": k1},
			wantPEKs:            map[string]key.Material{"pek-10": k1},
		},
		{
			name:                "stable state (before rotation)",
			initialBSKs:         map[string]key.Material{"bsk-10": k0, "bsk-20": k1},
			initialPEKs:         map[string]key.Material{"pek-20": k1},
			batchSigningKey:     k(kv(20, k1), kv(10, k0)),
			packetEncryptionKey: k(kv(20, k1), kv(10, k0)),
			wantBSKs:            map[string]key.Material{"bsk-10": k0, "bsk-20": k1},
			wantPEKs:            map[string]key.Material{"pek-20": k1},
		},
		{
			name:                "new key",
			initialBSKs:         map[string]key.Material{"bsk-10": k0, "bsk-20": k1},
			initialPEKs:         map[string]key.Material{"pek-20": k1},
			batchSigningKey:     k(kv(20, k1), kv(10, k0), kv(30, k2)),
			packetEncryptionKey: k(kv(20, k1), kv(10, k0), kv(30, k2)),
			wantBSKs:            map[string]key.Material{"bsk-10": k0, "bsk-20": k1, "bsk-30": k2},
			wantPEKs:            map[string]key.Material{"pek-20": k1},
		},
		{
			name:                "rotation",
			initialBSKs:         map[string]key.Material{"bsk-10": k0, "bsk-20": k1, "bsk-30": k2},
			initialPEKs:         map[string]key.Material{"pek-20": k1},
			batchSigningKey:     k(kv(30, k2), kv(10, k0), kv(20, k1)),
			packetEncryptionKey: k(kv(30, k2), kv(10, k0), kv(20, k1)),
			wantBSKs:            map[string]key.Material{"bsk-10": k0, "bsk-20": k1, "bsk-30": k2},
			wantPEKs:            map[string]key.Material{"pek-30": k2},
		},
		{
			name:                "removal",
			initialBSKs:         map[string]key.Material{"bsk-10": k0, "bsk-20": k1, "bsk-30": k2},
			initialPEKs:         map[string]key.Material{"pek-30": k2},
			batchSigningKey:     k(kv(30, k2), kv(20, k1)),
			packetEncryptionKey: k(kv(30, k2), kv(20, k1)),
			wantBSKs:            map[string]key.Material{"bsk-20": k1, "bsk-30": k2},
			wantPEKs:            map[string]key.Material{"pek-30": k2},
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()

			// Construct a manifest according to `initialBSKs` & `initialPEKs`.
			// Specify non-key fields so that we can check that they are carried through UpdateKeys unmodified.
			m := DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       fmt.Sprintf("%q ingestion identity", test.name),
				IngestionBucket:         fmt.Sprintf("%q ingestion bucket", test.name),
				PeerValidationIdentity:  fmt.Sprintf("%q peer validation identity", test.name),
				PeerValidationBucket:    fmt.Sprintf("%q peer validation bucket", test.name),
				BatchSigningPublicKeys:  BatchSigningPublicKeys{},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{},
			}
			origM := m // We save off a copy of our initial manifest so that we can check that it didn't change.
			origM.BatchSigningPublicKeys, origM.PacketEncryptionKeyCSRs = BatchSigningPublicKeys{}, PacketEncryptionKeyCSRs{}
			for kid, raw := range test.initialBSKs {
				pkix, err := raw.PublicAsPKIX()
				if err != nil {
					t.Fatalf("Couldn't serialize initialBSK %q as PKIX: %v", kid, err)
				}
				m.BatchSigningPublicKeys[kid] = BatchSigningPublicKey{PublicKey: pkix}
				origM.BatchSigningPublicKeys[kid] = BatchSigningPublicKey{PublicKey: pkix}
			}
			for kid, raw := range test.initialPEKs {
				csr, err := raw.PublicAsCSR("initial.fqdn")
				if err != nil {
					t.Fatalf("Couldn't serialize initialPEK %q as CSR: %v", kid, err)
				}
				m.PacketEncryptionKeyCSRs[kid] = PacketEncryptionCertificate{CertificateSigningRequest: csr}
				origM.PacketEncryptionKeyCSRs[kid] = PacketEncryptionCertificate{CertificateSigningRequest: csr}
			}

			// Perform an update-keys operation.
			gotM, err := m.UpdateKeys(UpdateKeysConfig{
				BatchSigningKey:             test.batchSigningKey,
				BatchSigningKeyIDPrefix:     bskPrefix,
				PacketEncryptionKey:         test.packetEncryptionKey,
				PacketEncryptionKeyIDPrefix: pekPrefix,
				PacketEncryptionKeyCSRFQDN:  fqdn,
			})
			if err != nil {
				t.Fatalf("Unexpected error from UpdateKeys: %v", err)
			}

			// Check that we didn't modify the initial manifest.
			if diff := cmp.Diff(origM, m); diff != "" {
				t.Errorf("UpdateKeys modified its receiver (-orig +new):\n%s", diff)
			}

			// Check that non-key fields are copied without modification.
			gotBSKs, gotPEKs := gotM.BatchSigningPublicKeys, gotM.PacketEncryptionKeyCSRs
			m.BatchSigningPublicKeys, m.PacketEncryptionKeyCSRs = nil, nil
			gotM.BatchSigningPublicKeys, gotM.PacketEncryptionKeyCSRs = nil, nil
			if diff := cmp.Diff(m, gotM); diff != "" {
				t.Errorf("UpdateKeys modified non-key fields (-want +got)\n%s", diff)
			}

			// Check that keys are as expected by parsing the keys out of the
			// manifest & checking that they are equal to what is expected. (We
			// have to do it this way because not all methods of serializing a
			// public key are deterministic, i.e. repeatedly serializing a
			// public key into a CSR will produce different bytes each time.)
			wantBSKPubkeys, wantPEKPubkeys := map[string]*ecdsa.PublicKey{}, map[string]*ecdsa.PublicKey{}
			for kid, bsk := range test.wantBSKs {
				wantBSKPubkeys[kid] = pubkeyFromRaw(bsk)
			}
			for kid, pek := range test.wantPEKs {
				wantPEKPubkeys[kid] = pubkeyFromRaw(pek)
			}

			gotBSKPubkeys, gotPEKPubkeys := map[string]*ecdsa.PublicKey{}, map[string]*ecdsa.PublicKey{}
			for kid, bsk := range gotBSKs {
				gotBSKPubkeys[kid] = pubkeyFromPKIX(bsk.PublicKey)
			}
			for kid, pek := range gotPEKs {
				gotPEKPubkeys[kid] = pubkeyFromCSR(pek.CertificateSigningRequest)
			}

			if diff := cmp.Diff(wantBSKPubkeys, gotBSKPubkeys); diff != "" {
				t.Errorf("UpdateKeys produced incorrect batch signing keys (-want +got):\n%s", diff)
			}
			if diff := cmp.Diff(wantPEKPubkeys, gotPEKPubkeys); diff != "" {
				t.Errorf("UpdateKeys produced incorrect packet encryption keys (-want +got):\n%s", diff)
			}
		})
	}

	// Failure tests.
	for _, test := range []struct {
		name       string
		cfg        UpdateKeysConfig
		wantErrStr string
	}{
		{
			name: "empty batch signing key",
			cfg: UpdateKeysConfig{
				BatchSigningKey:             key.Key{},
				BatchSigningKeyIDPrefix:     bskPrefix,
				PacketEncryptionKey:         k(kv(0, k0)),
				PacketEncryptionKeyIDPrefix: pekPrefix,
				PacketEncryptionKeyCSRFQDN:  fqdn,
			},
			wantErrStr: "batch signing key has no key versions",
		},
		{
			name: "empty packet encryption key",
			cfg: UpdateKeysConfig{
				BatchSigningKey:             k(kv(0, k0)),
				BatchSigningKeyIDPrefix:     bskPrefix,
				PacketEncryptionKey:         key.Key{},
				PacketEncryptionKeyIDPrefix: pekPrefix,
				PacketEncryptionKeyCSRFQDN:  fqdn,
			},
			wantErrStr: "packet encryption key has no key versions",
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()
			_, err := DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       fmt.Sprintf("%q ingestion identity", test.name),
				IngestionBucket:         fmt.Sprintf("%q ingestion bucket", test.name),
				PeerValidationIdentity:  fmt.Sprintf("%q peer validation identity", test.name),
				PeerValidationBucket:    fmt.Sprintf("%q peer validation bucket", test.name),
				BatchSigningPublicKeys:  BatchSigningPublicKeys{},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{},
			}.UpdateKeys(test.cfg)
			if err == nil || !strings.Contains(err.Error(), test.wantErrStr) {
				t.Errorf("Wanted error containing %q, got: %v", test.wantErrStr, err)
			}
		})
	}
}

func TestUpdateKeysValidations(t *testing.T) {
	t.Parallel()

	// We check only validation check failures here, and we do so by directly
	// calling `validateUpdatedManifest` because there is no/should be no way
	// to trigger these checks via UpdateKeys.

	for _, test := range []struct {
		name        string
		manifest    DataShareProcessorSpecificManifest
		oldManifest DataShareProcessorSpecificManifest // if unspecified, same as manifest
		wantErrStr  string
	}{
		{
			name: "no batch signing key",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			wantErrStr: "no batch signing",
		},
		{
			name: "no packet encryption key",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{},
			},
			wantErrStr: "exactly one packet encryption",
		},
		{
			name: "multiple packet encryption keys",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek-1": PacketEncryptionCertificate{CertificateSigningRequest: "csr-1"},
					"pek-2": PacketEncryptionCertificate{CertificateSigningRequest: "csr-2"},
				},
			},
			wantErrStr: "exactly one packet encryption",
		},
		{
			name: "non-key data modified (format)",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 2,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name: "non-key data modified (ingestion identity)",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "modified-ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name: "non-key data modified (ingestion bucket)",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "modified-ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name: "non-key data modified (peer validation identity)",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "modified-peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name: "non-key data modified (peer validation bucket)",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "modified-peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name: "existing batch signing key modified",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "modified-pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
			wantErrStr: "pre-existing batch signing key",
		},
		{
			name: "existing packet encryption key modified",
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "modified-csr"},
				},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{
					"bsk": BatchSigningPublicKey{PublicKey: "pubkey"},
				},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek": PacketEncryptionCertificate{CertificateSigningRequest: "csr"},
				},
			},
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()
			oldM := test.oldManifest
			if oldM.Equal(DataShareProcessorSpecificManifest{}) {
				oldM = test.manifest
			}
			err := validateUpdatedManifest(test.manifest, oldM)
			if err == nil || !strings.Contains(err.Error(), test.wantErrStr) {
				t.Errorf("Wanted error containing %q, got: %v", test.wantErrStr, err)
			}
		})
	}
}

// mustP256 creates a new random P256 key or dies trying.
func mustP256() key.Material {
	k, err := key.P256.New()
	if err != nil {
		panic(fmt.Sprintf("Couldn't create new P256 key: %v", err))
	}
	return k
}

// k creates a new key or dies trying. pkv is the primary key version, vs are
// other versions.
func k(pkv key.Version, vs ...key.Version) key.Key {
	k, err := key.FromVersions(pkv, vs...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key from versions: %v", err))
	}
	return k
}

// kv creates a key version with the given timestamp and raw key.
func kv(ts int64, k key.Material) key.Version {
	return key.Version{
		KeyMaterial:       k,
		CreationTimestamp: ts,
	}
}

func pubkeyFromRaw(raw key.Material) *ecdsa.PublicKey {
	// A raw key won't give us its key material (even public) directly, but we
	// can serialize it & parse it back.
	pkix, err := raw.PublicAsPKIX()
	if err != nil {
		panic(fmt.Sprintf("Couldn't serialize raw key as PEM-encoded PKIX: %v", err))
	}
	return pubkeyFromPKIX(pkix)
}

func pubkeyFromPKIX(pemPKIX string) *ecdsa.PublicKey {
	p, _ := pem.Decode([]byte(pemPKIX))
	if p == nil {
		panic(fmt.Sprintf("Couldn't decode PEM block from %q", pemPKIX))
	}
	pub, err := x509.ParsePKIXPublicKey(p.Bytes)
	if err != nil {
		panic(fmt.Sprintf("Couldn't parse PEM block data as PKIX public key: %v", err))
	}
	return pub.(*ecdsa.PublicKey)
}

func pubkeyFromCSR(pemCSR string) *ecdsa.PublicKey {
	p, _ := pem.Decode([]byte(pemCSR))
	if p == nil {
		panic(fmt.Sprintf("Couldn't decode PEM block from %q", pemCSR))
	}
	csr, err := x509.ParseCertificateRequest(p.Bytes)
	if err != nil {
		panic(fmt.Sprintf("Couldn't parse PEM block data as a CSR: %v", err))
	}
	if err := csr.CheckSignature(); err != nil {
		panic("CSR is unsigned!?")
	}
	return csr.PublicKey.(*ecdsa.PublicKey)
}
