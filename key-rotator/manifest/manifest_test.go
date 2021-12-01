package manifest

import (
	"crypto/ecdsa"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"

	"github.com/abetterinternet/prio-server/key-rotator/key"
	keytest "github.com/abetterinternet/prio-server/key-rotator/key/test"
)

const (
	bskPrefix = "bsk"
	pekPrefix = "pek"
	fqdn      = "arbitrary.fqdn"
)

func TestUpdateKeys(t *testing.T) {
	t.Parallel()

	// Success tests.
	for _, test := range []struct {
		name string

		// Initial manifest parameters.
		initialBSKs BatchSigningPublicKeys
		initialPEKs PacketEncryptionKeyCSRs

		// UpdateKeys parameters.
		batchSigningKey     key.Key
		packetEncryptionKey key.Key

		// Desired output manifest parameters.
		wantBSKs BatchSigningPublicKeys
		wantPEKs PacketEncryptionKeyCSRs
	}{
		{
			name:                "no keys at start (new environment rollout)",
			batchSigningKey:     bsk(15, 10, 20),
			packetEncryptionKey: pek(20, 10, 15),
			wantBSKs:            manifestBSK(10, 15, 20),
			wantPEKs:            manifestPEK(20),
		},

		// the following tests verify how manifest updates behave against a
		// simulated rotation of keys
		{
			name:                "before first rotation (0 timestamp)",
			initialBSKs:         manifestBSK(0),
			initialPEKs:         manifestPEK(0),
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			wantBSKs:            manifestBSK(0),
			wantPEKs:            manifestPEK(0),
		},
		{
			name:                "first new key (0 timestamp)",
			initialBSKs:         manifestBSK(0),
			initialPEKs:         manifestPEK(0),
			batchSigningKey:     bsk(0, 10),
			packetEncryptionKey: pek(0, 10),
			wantBSKs:            manifestBSK(0, 10),
			wantPEKs:            manifestPEK(0),
		},
		{
			name:                "first primary-key change (0 timestamp)",
			initialBSKs:         manifestBSK(0, 10),
			initialPEKs:         manifestPEK(0),
			batchSigningKey:     bsk(10, 0),
			packetEncryptionKey: pek(10, 0),
			wantBSKs:            manifestBSK(0, 10),
			wantPEKs:            manifestPEK(10),
		},
		{
			name:                "first key removal (0 timestamp)",
			initialBSKs:         manifestBSK(0, 10),
			initialPEKs:         manifestPEK(10),
			batchSigningKey:     bsk(10),
			packetEncryptionKey: pek(10),
			wantBSKs:            manifestBSK(10),
			wantPEKs:            manifestPEK(10),
		},
		{
			name:                "stable state (before rotation)",
			initialBSKs:         manifestBSK(10, 20),
			initialPEKs:         manifestPEK(20),
			batchSigningKey:     bsk(20, 10),
			packetEncryptionKey: pek(20, 10),
			wantBSKs:            manifestBSK(10, 20),
			wantPEKs:            manifestPEK(20),
		},
		{
			name:                "new key",
			initialBSKs:         manifestBSK(10, 20),
			initialPEKs:         manifestPEK(20),
			batchSigningKey:     bsk(20, 10, 30),
			packetEncryptionKey: pek(20, 10, 30),
			wantBSKs:            manifestBSK(10, 20, 30),
			wantPEKs:            manifestPEK(20),
		},
		{
			name:                "rotation",
			initialBSKs:         manifestBSK(10, 20, 30),
			initialPEKs:         manifestPEK(20),
			batchSigningKey:     bsk(30, 10, 20),
			packetEncryptionKey: pek(30, 10, 20),
			wantBSKs:            manifestBSK(10, 20, 30),
			wantPEKs:            manifestPEK(30),
		},
		{
			name:                "removal",
			initialBSKs:         manifestBSK(10, 20, 30),
			initialPEKs:         manifestPEK(30),
			batchSigningKey:     bsk(30, 20),
			packetEncryptionKey: pek(30, 20),
			wantBSKs:            manifestBSK(20, 30),
			wantPEKs:            manifestPEK(30),
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
			for kid, bspk := range test.initialBSKs {
				m.BatchSigningPublicKeys[kid] = bspk
				origM.BatchSigningPublicKeys[kid] = bspk
			}
			for kid, pek := range test.initialPEKs {
				m.PacketEncryptionKeyCSRs[kid] = pek
				origM.PacketEncryptionKeyCSRs[kid] = pek
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
				pub, err := bsk.toPublicKey()
				if err != nil {
					t.Errorf("Couldn't convert wantBSKs[%q] to public key: %v", kid, err)
				}
				wantBSKPubkeys[kid] = pub
			}
			for kid, pek := range test.wantPEKs {
				pub, err := pek.toPublicKey()
				if err != nil {
					t.Errorf("Couldn't convert wantPEKs[%q] to public key: %v", kid, err)
				}
				wantPEKPubkeys[kid] = pub
			}

			gotBSKPubkeys, gotPEKPubkeys := map[string]*ecdsa.PublicKey{}, map[string]*ecdsa.PublicKey{}
			for kid, bsk := range gotBSKs {
				pub, err := bsk.toPublicKey()
				if err != nil {
					t.Errorf("Batch signing key ID %q was unparseable: %v", kid, err)
					continue
				}
				if _, err := time.Parse(time.RFC3339, bsk.Expiration); err != nil {
					t.Errorf("Batch singing key ID %q had unparseable expiration %q: %v", kid, bsk.Expiration, err)
				}
				gotBSKPubkeys[kid] = pub
			}
			for kid, pek := range gotPEKs {
				pub, err := pek.toPublicKey()
				if err != nil {
					t.Errorf("Packet encryption key ID %q was unparseable: %v", kid, err)
					continue
				}
				gotPEKPubkeys[kid] = pub
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
		name                string
		manifestBSKs        BatchSigningPublicKeys
		manifestPEKs        PacketEncryptionKeyCSRs
		batchSigningKey     key.Key
		packetEncryptionKey key.Key
		wantErrStr          string
	}{
		{
			name:                "empty batch signing key",
			batchSigningKey:     bsk(),
			packetEncryptionKey: pek(0),
			wantErrStr:          "batch signing key has no key versions",
		},
		{
			name:                "empty packet encryption key",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(),
			wantErrStr:          "packet encryption key has no key versions",
		},
		{
			name:                "manifest does not contain primary version of update config batch signing key",
			manifestBSKs:        manifestBSK(10, 20),
			manifestPEKs:        manifestPEK(0),
			batchSigningKey:     bsk(0, 10, 20),
			packetEncryptionKey: pek(0),
			wantErrStr:          "update's batch signing key primary version",
		},
		{
			name:                "manifest contains packet encryption key that does not match to update config packet encryption key",
			manifestBSKs:        manifestBSK(0),
			manifestPEKs:        manifestPEK(10),
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			wantErrStr:          "manifest packet encryption key version",
		},
		{
			name:                "key material differs from update config key material (batch signing key)",
			manifestBSKs:        BatchSigningPublicKeys{"bsk": batchSigningPublicKey(keytest.Material("bsk-garbage"))},
			manifestPEKs:        manifestPEK(0),
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			wantErrStr:          "key mismatch in batch signing key",
		},
		{
			name:                "key material differs from update config key material (packet encryption key)",
			manifestBSKs:        manifestBSK(0),
			manifestPEKs:        PacketEncryptionKeyCSRs{"pek": packetEncryptionCertificate(keytest.Material("pek-garbage"))},
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			wantErrStr:          "key mismatch in packet encryption key",
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()

			// Create manifest per test parameters.
			m := DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       fmt.Sprintf("%q ingestion identity", test.name),
				IngestionBucket:         fmt.Sprintf("%q ingestion bucket", test.name),
				PeerValidationIdentity:  fmt.Sprintf("%q peer validation identity", test.name),
				PeerValidationBucket:    fmt.Sprintf("%q peer validation bucket", test.name),
				BatchSigningPublicKeys:  test.manifestBSKs,
				PacketEncryptionKeyCSRs: test.manifestPEKs,
			}

			// Create config per test paramemters.
			cfg := UpdateKeysConfig{
				BatchSigningKey:             test.batchSigningKey,
				BatchSigningKeyIDPrefix:     bskPrefix,
				PacketEncryptionKey:         test.packetEncryptionKey,
				PacketEncryptionKeyIDPrefix: pekPrefix,
				PacketEncryptionKeyCSRFQDN:  fqdn,
			}

			// Attempt to update keys, and verify that we get the expected error.
			_, err := m.UpdateKeys(cfg)
			if err == nil || !strings.Contains(err.Error(), test.wantErrStr) {
				t.Errorf("Wanted error containing %q, got: %v", test.wantErrStr, err)
			}
		})
	}
}

func TestPostUpdateKeysValidations(t *testing.T) {
	t.Parallel()

	// We check only validation check failures here, and we do so by directly
	// calling `validateUpdatedManifest` rather than calling `UpdateKeys`
	// because there is no/should be no way to trigger these checks via
	// UpdateKeys. Successes are implicitly tested by testing the public
	// `UpdateKeys` API in `TestUpdateKeys`.

	for _, test := range []struct {
		name                string
		batchSigningKey     key.Key
		packetEncryptionKey key.Key
		manifest            DataShareProcessorSpecificManifest
		oldManifest         DataShareProcessorSpecificManifest // if unspecified, same as manifest
		wantErrStr          string
	}{
		{
			name:                "no batch signing key",
			batchSigningKey:     bsk(),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "no batch signing",
		},
		{
			name:                "manifest batch signing key includes unexpected version",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0, 10),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "manifest included unexpected batch signing key",
		},
		{
			name:                "manifest batch signing key missing expected version",
			batchSigningKey:     bsk(0, 10),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(10),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "manifest missing expected batch signing key",
		},
		{
			name:                "no packet encryption key",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(),
			},
			wantErrStr: "exactly one packet encryption",
		},
		{
			name:                "mismatched packet encryption key",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(10),
			},
			wantErrStr: "manifest included unexpected packet encryption key",
		},
		{
			name:                "multiple packet encryption keys",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(1, 2),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(1, 2),
			},
			wantErrStr: "exactly one packet encryption",
		},
		{
			name:                "non-key data modified (format)",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  2,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (ingestion identity)",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "modified-ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (ingestion bucket)",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "modified-ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (peer validation identity)",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "modified-peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (peer validation bucket)",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "modified-peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "existing batch signing key modified",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": BatchSigningPublicKey{PublicKey: "old BSK value"}},
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			wantErrStr: "pre-existing batch signing key",
		},
		{
			name:                "existing packet encryption key modified",
			batchSigningKey:     bsk(0),
			packetEncryptionKey: pek(0),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: manifestPEK(0),
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  manifestBSK(0),
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": PacketEncryptionCertificate{CertificateSigningRequest: "old PEK value"}},
			},
			wantErrStr: "pre-existing packet encryption key",
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()
			cfg := UpdateKeysConfig{
				BatchSigningKey:             test.batchSigningKey,
				BatchSigningKeyIDPrefix:     bskPrefix,
				PacketEncryptionKey:         test.packetEncryptionKey,
				PacketEncryptionKeyIDPrefix: pekPrefix,
				PacketEncryptionKeyCSRFQDN:  fqdn,
			}
			oldM := test.oldManifest
			if oldM.Equal(DataShareProcessorSpecificManifest{}) {
				oldM = test.manifest
			}
			err := validatePostUpdateManifest(cfg, test.manifest, oldM)
			if err == nil || !strings.Contains(err.Error(), test.wantErrStr) {
				t.Errorf("Wanted error containing %q, got: %v", test.wantErrStr, err)
			}
		})
	}
}

// batchSigningPublicKey creates a BatchSigningPublicKey containing the public
// portion of the given key material.
func batchSigningPublicKey(m key.Material) BatchSigningPublicKey {
	pkix, err := m.PublicAsPKIX()
	if err != nil {
		panic(fmt.Sprintf("Couldn't convert public key to PKIX: %v", err))
	}
	return BatchSigningPublicKey{PublicKey: pkix}
}

// packetEncryptionCertificate creates a PacketEncryptionCertificate containing
// the public portion of the given key material.
func packetEncryptionCertificate(m key.Material) PacketEncryptionCertificate {
	csr, err := m.PublicAsCSR(fqdn)
	if err != nil {
		panic(fmt.Sprintf("Couldn't convert public key to CSR: %v", err))
	}
	return PacketEncryptionCertificate{CertificateSigningRequest: csr}
}

// bsk creates a batch signing key with the given timestamps. Key material is
// arbitrary, but will match that of other batch signing keys at the same
// timestamp, and will very likely not match other key materials. If timestamps
// are provided, the first timestamp is used as the primary key version.
func bsk(tss ...int64) key.Key {
	if len(tss) == 0 {
		return key.Key{}
	}
	var vs []key.Version
	for _, ts := range tss {
		vs = append(vs, key.Version{
			KeyMaterial:       keytest.Material(bskKID(ts)),
			CreationTimestamp: ts,
		})
	}
	k, err := key.FromVersions(vs[0], vs[1:]...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key: %v", err))
	}
	return k
}

// pek creates a packet encryption key with the given timestamps. Key material
// is arbitrary, but will match that of other packet encryption keys at the
// same timestamp, and will very likely not match other key materials. If
// timestamps are provided, the first timestamp is used as the primary key
// version.
func pek(tss ...int64) key.Key {
	if len(tss) == 0 {
		return key.Key{}
	}
	var vs []key.Version
	for _, ts := range tss {
		vs = append(vs, key.Version{
			KeyMaterial:       keytest.Material(pekKID(ts)),
			CreationTimestamp: ts,
		})
	}
	k, err := key.FromVersions(vs[0], vs[1:]...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key: %v", err))
	}
	return k
}

// manifestBSK creates a manifest BatchSigningPublicKeys with the given
// timetamps. Order does not matter. Key material is arbitrary, but will match
// that of other batch signing keys at the same timestamp, and will very likely
// not match other key materials.
func manifestBSK(tss ...int64) BatchSigningPublicKeys {
	rslt := BatchSigningPublicKeys{}
	for _, ts := range tss {
		kid := bskKID(ts)
		pkix, err := keytest.Material(kid).PublicAsPKIX()
		if err != nil {
			panic(fmt.Sprintf("Couldn't serialize key material as PKIX: %v", err))
		}
		rslt[kid] = BatchSigningPublicKey{
			PublicKey:  pkix,
			Expiration: time.Now().Format(time.RFC3339),
		}
	}
	return rslt
}

// manifestPEK creates a manifest PacketEncryptionKeyCSRs with the given
// timestamps. Order does not matter. Key material is arbitrary, but will match
// that of other packet encryption keys at the same timestamp, and will very
// likely not match other key materials.
func manifestPEK(tss ...int64) PacketEncryptionKeyCSRs {
	rslt := PacketEncryptionKeyCSRs{}
	for _, ts := range tss {
		kid := pekKID(ts)
		csr, err := keytest.Material(kid).PublicAsCSR(fqdn)
		if err != nil {
			panic(fmt.Sprintf("Couldn't serialize key material as CSR: %v", err))
		}
		rslt[kid] = PacketEncryptionCertificate{CertificateSigningRequest: csr}
	}
	return rslt
}

func bskKID(ts int64) string {
	if ts == 0 {
		return bskPrefix
	}
	return fmt.Sprintf("%s-%d", bskPrefix, ts)
}

func pekKID(ts int64) string {
	if ts == 0 {
		return pekPrefix
	}
	return fmt.Sprintf("%s-%d", pekPrefix, ts)
}
