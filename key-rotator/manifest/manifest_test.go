package manifest

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"fmt"
	"hash/fnv"
	"io"
	"math/rand"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

const fqdn = "arbitrary.fqdn"

func TestUpdateKeys(t *testing.T) {
	t.Parallel()

	const (
		bskPrefix = "bsk"
		pekPrefix = "pek"
	)

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
			batchSigningKey:     k(kv(15, m("bsk-15")), kv(10, m("bsk-10")), kv(20, m("bsk-20"))),
			packetEncryptionKey: k(kv(20, m("pek-20")), kv(10, m("pek-10")), kv(15, m("pek-15"))),
			wantBSKs: map[string]key.Material{
				"bsk-10": m("bsk-10"),
				"bsk-15": m("bsk-15"),
				"bsk-20": m("bsk-20"),
			},
			wantPEKs: map[string]key.Material{
				"pek-20": m("pek-20"),
			},
		},

		// the following tests verify how manifest updates behave against a
		// simulated rotation of keys
		{
			name:                "before first rotation (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": m("bsk")},
			initialPEKs:         map[string]key.Material{"pek": m("pek")},
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			wantBSKs:            map[string]key.Material{"bsk": m("bsk")},
			wantPEKs:            map[string]key.Material{"pek": m("pek")},
		},
		{
			name:                "first new key (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": m("bsk")},
			initialPEKs:         map[string]key.Material{"pek": m("pek")},
			batchSigningKey:     k(kv(0, m("bsk")), kv(10, m("bsk-10"))),
			packetEncryptionKey: k(kv(0, m("pek")), kv(10, m("pek-10"))),
			wantBSKs:            map[string]key.Material{"bsk": m("bsk"), "bsk-10": m("bsk-10")},
			wantPEKs:            map[string]key.Material{"pek": m("pek")},
		},
		{
			name:                "first primary-key change (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": m("bsk"), "bsk-10": m("bsk-10")},
			initialPEKs:         map[string]key.Material{"pek": m("pek")},
			batchSigningKey:     k(kv(10, m("bsk-10")), kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(10, m("pek-10")), kv(0, m("pek"))),
			wantBSKs:            map[string]key.Material{"bsk": m("bsk"), "bsk-10": m("bsk-10")},
			wantPEKs:            map[string]key.Material{"pek-10": m("pek-10")},
		},
		{
			name:                "first key removal (0 timestamp)",
			initialBSKs:         map[string]key.Material{"bsk": m("bsk"), "bsk-10": m("bsk-10")},
			initialPEKs:         map[string]key.Material{"pek-10": m("pek-10")},
			batchSigningKey:     k(kv(10, m("bsk-10"))),
			packetEncryptionKey: k(kv(10, m("pek-10"))),
			wantBSKs:            map[string]key.Material{"bsk-10": m("bsk-10")},
			wantPEKs:            map[string]key.Material{"pek-10": m("pek-10")},
		},
		{
			name:                "stable state (before rotation)",
			initialBSKs:         map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20")},
			initialPEKs:         map[string]key.Material{"pek-20": m("pek-20")},
			batchSigningKey:     k(kv(20, m("bsk-20")), kv(10, m("bsk-10"))),
			packetEncryptionKey: k(kv(20, m("pek-20")), kv(10, m("pek-10"))),
			wantBSKs:            map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20")},
			wantPEKs:            map[string]key.Material{"pek-20": m("pek-20")},
		},
		{
			name:                "new key",
			initialBSKs:         map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20")},
			initialPEKs:         map[string]key.Material{"pek-20": m("pek-20")},
			batchSigningKey:     k(kv(20, m("bsk-20")), kv(10, m("bsk-10")), kv(30, m("bsk-30"))),
			packetEncryptionKey: k(kv(20, m("pek-20")), kv(10, m("pek-10")), kv(30, m("pek-30"))),
			wantBSKs:            map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20"), "bsk-30": m("bsk-30")},
			wantPEKs:            map[string]key.Material{"pek-20": m("pek-20")},
		},
		{
			name:                "rotation",
			initialBSKs:         map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20"), "bsk-30": m("bsk-30")},
			initialPEKs:         map[string]key.Material{"pek-20": m("pek-20")},
			batchSigningKey:     k(kv(30, m("bsk-30")), kv(10, m("bsk-10")), kv(20, m("bsk-20"))),
			packetEncryptionKey: k(kv(30, m("pek-30")), kv(10, m("pek-10")), kv(20, m("pek-20"))),
			wantBSKs:            map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20"), "bsk-30": m("bsk-30")},
			wantPEKs:            map[string]key.Material{"pek-30": m("pek-30")},
		},
		{
			name:                "removal",
			initialBSKs:         map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20"), "bsk-30": m("bsk-30")},
			initialPEKs:         map[string]key.Material{"pek-30": m("pek-30")},
			batchSigningKey:     k(kv(30, m("bsk-30")), kv(20, m("bsk-20"))),
			packetEncryptionKey: k(kv(30, m("pek-30")), kv(20, m("pek-20"))),
			wantBSKs:            map[string]key.Material{"bsk-20": m("bsk-20"), "bsk-30": m("bsk-30")},
			wantPEKs:            map[string]key.Material{"pek-30": m("pek-30")},
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
			for kid, keyMaterial := range test.initialBSKs {
				pkix, err := keyMaterial.PublicAsPKIX()
				if err != nil {
					t.Fatalf("Couldn't serialize initialBSK %q as PKIX: %v", kid, err)
				}
				m.BatchSigningPublicKeys[kid] = BatchSigningPublicKey{PublicKey: pkix}
				origM.BatchSigningPublicKeys[kid] = BatchSigningPublicKey{PublicKey: pkix}
			}
			for kid, keyMaterial := range test.initialPEKs {
				csr, err := keyMaterial.PublicAsCSR("initial.fqdn")
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
				wantBSKPubkeys[kid] = bsk.Public()
			}
			for kid, pek := range test.wantPEKs {
				wantPEKPubkeys[kid] = pek.Public()
			}

			gotBSKPubkeys, gotPEKPubkeys := map[string]*ecdsa.PublicKey{}, map[string]*ecdsa.PublicKey{}
			for kid, bsk := range gotBSKs {
				pub, err := bsk.toPublicKey()
				if err != nil {
					t.Errorf("Batch signing key ID %q was unparseable: %v", kid, err)
					continue
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
		manifestBSKs        map[string]key.Material
		manifestPEKs        map[string]key.Material
		batchSigningKey     key.Key
		packetEncryptionKey key.Key
		cfg                 UpdateKeysConfig
		wantErrStr          string
	}{
		{
			name:                "empty batch signing key",
			batchSigningKey:     key.Key{},
			packetEncryptionKey: k(kv(0, m("pek"))),
			wantErrStr:          "batch signing key has no key versions",
		},
		{
			name:                "empty packet encryption key",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: key.Key{},
			wantErrStr:          "packet encryption key has no key versions",
		},
		{
			name:                "manifest does not contain primary version of update config batch signing key",
			manifestBSKs:        map[string]key.Material{"bsk-10": m("bsk-10"), "bsk-20": m("bsk-20")},
			manifestPEKs:        map[string]key.Material{"pek": m("pek")},
			batchSigningKey:     k(kv(0, m("bsk")), kv(10, m("bsk-10")), kv(20, m("bsk-20"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			wantErrStr:          "update's batch signing key primary version",
		},
		{
			name:                "manifest contains packet encryption key that does not match to update config packet encryption key",
			manifestBSKs:        map[string]key.Material{"bsk": m("bsk")},
			manifestPEKs:        map[string]key.Material{"pek-10": m("pek-10")},
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			wantErrStr:          "manifest packet encryption key version",
		},
		{
			name:                "key material differs from update config key material (batch signing key)",
			manifestBSKs:        map[string]key.Material{"bsk": m("bsk-garbage")},
			manifestPEKs:        map[string]key.Material{"pek": m("pek")},
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			wantErrStr:          "key mismatch in batch signing key",
		},
		{
			name:                "key material differs from update config key material (packet encryption key)",
			manifestBSKs:        map[string]key.Material{"bsk": m("bsk")},
			manifestPEKs:        map[string]key.Material{"pek": m("pek-garbage")},
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
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
				BatchSigningPublicKeys:  BatchSigningPublicKeys{},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{},
			}
			for kid, keyMaterial := range test.manifestBSKs {
				pkix, err := keyMaterial.PublicAsPKIX()
				if err != nil {
					t.Fatalf("Couldn't serialize initialBSK %q as PKIX: %v", kid, err)
				}
				m.BatchSigningPublicKeys[kid] = BatchSigningPublicKey{PublicKey: pkix}
			}
			for kid, keyMaterial := range test.manifestPEKs {
				csr, err := keyMaterial.PublicAsCSR("initial.fqdn")
				if err != nil {
					t.Fatalf("Couldn't serialize initialPEK %q as CSR: %v", kid, err)
				}
				m.PacketEncryptionKeyCSRs[kid] = PacketEncryptionCertificate{CertificateSigningRequest: csr}
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
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "no batch signing",
		},
		{
			name:                "manifest batch signing key includes unexpected version",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk")), "bsk-10": bsk(m("bsk-10"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "manifest included unexpected batch signing key",
		},
		{
			name:                "manifest batch signing key missing expected version",
			batchSigningKey:     k(kv(0, m("bsk")), kv(10, m("bsk-10"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk-10": bsk(m("bsk-10"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "manifest missing expected batch signing key",
		},
		{
			name:            "no packet encryption key",
			batchSigningKey: k(kv(0, m("bsk"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{},
			},
			wantErrStr: "exactly one packet encryption",
		},
		{
			name:                "mismatched packet encryption key",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek-10": pek(m("pek-10"))},
			},
			wantErrStr: "manifest included unexpected packet encryption key",
		},
		{
			name:                "multiple packet encryption keys",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(1, m("pek-1")), kv(2, m("pek-2"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                 1,
				IngestionIdentity:      "ingestion-identity",
				IngestionBucket:        "ingestion-bucket",
				PeerValidationIdentity: "peer-validation-identity",
				PeerValidationBucket:   "peer-validation-bucket",
				BatchSigningPublicKeys: BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{
					"pek-1": pek(m("pek-1")),
					"pek-2": pek(m("pek-2")),
				},
			},
			wantErrStr: "exactly one packet encryption",
		},
		{
			name:                "non-key data modified (format)",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  2,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (ingestion identity)",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "modified-ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (ingestion bucket)",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "modified-ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (peer validation identity)",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "modified-peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "non-key data modified (peer validation bucket)",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "modified-peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "non-key data modified",
		},
		{
			name:                "existing batch signing key modified",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": BatchSigningPublicKey{PublicKey: "old BSK value"}},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			wantErrStr: "pre-existing batch signing key",
		},
		{
			name:                "existing packet encryption key modified",
			batchSigningKey:     k(kv(0, m("bsk"))),
			packetEncryptionKey: k(kv(0, m("pek"))),
			manifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
				PacketEncryptionKeyCSRs: PacketEncryptionKeyCSRs{"pek": pek(m("pek"))},
			},
			oldManifest: DataShareProcessorSpecificManifest{
				Format:                  1,
				IngestionIdentity:       "ingestion-identity",
				IngestionBucket:         "ingestion-bucket",
				PeerValidationIdentity:  "peer-validation-identity",
				PeerValidationBucket:    "peer-validation-bucket",
				BatchSigningPublicKeys:  BatchSigningPublicKeys{"bsk": bsk(m("bsk"))},
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
				BatchSigningKeyIDPrefix:     "bsk",
				PacketEncryptionKey:         test.packetEncryptionKey,
				PacketEncryptionKeyIDPrefix: "pek",
				PacketEncryptionKeyCSRFQDN:  "unused.fqdn",
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

// m generates deterministic key material based on the given `kid`. It is very
// likely that different `kid` values will produce different key material. Not
// secure, for testing use only.
func m(kid string) key.Material {
	// Stretch `kid` into a deterministic, arbitrary stream of bytes.
	h := fnv.New64()
	io.WriteString(h, kid)
	rnd := rand.New(rand.NewSource(int64(h.Sum64())))

	// Use byte stream to generate a P256 key, and wrap it into a key.Material.
	privKey, err := ecdsa.GenerateKey(elliptic.P256(), rnd)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create new P256 key: %v", err))
	}
	m, err := key.P256MaterialFrom(privKey)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create P256 material: %v", err))
	}
	return m
}

func bsk(m key.Material) BatchSigningPublicKey {
	pkix, err := m.PublicAsPKIX()
	if err != nil {
		panic(fmt.Sprintf("Couldn't serialize key material as PKIX: %v", err))
	}
	return BatchSigningPublicKey{PublicKey: pkix}
}

func pek(m key.Material) PacketEncryptionCertificate {
	csr, err := m.PublicAsCSR(fqdn)
	if err != nil {
		panic(fmt.Sprintf("Couldn't serialize key material as CSR: %v", err))
	}
	return PacketEncryptionCertificate{CertificateSigningRequest: csr}
}
