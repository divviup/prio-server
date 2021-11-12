package manifest

import (
	"crypto/ecdsa"
	"crypto/x509"
	"encoding/pem"
	"fmt"
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
