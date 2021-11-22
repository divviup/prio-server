package main

import (
	"context"
	"fmt"
	"testing"
	"time"

	"github.com/abetterinternet/prio-server/key-rotator/key"
	keytest "github.com/abetterinternet/prio-server/key-rotator/key/test"
	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	storagetest "github.com/abetterinternet/prio-server/key-rotator/storage/test"
)

var ctx = context.Background()

type LI = storagetest.LocalityIngestor

type manifestInfo struct {
	batchSigningKeyVersions     []int64
	packetEncryptionKeyVersions []int64
}

func TestRotateKeys(t *testing.T) {
	t.Parallel()

	// Base configuration template.
	rotateKeysCFG := rotateKeysConfig{
		now:             time.Unix(100000, 0),
		locality:        "asgard",
		ingestors:       []string{"ingestor-1", "ingestor-2"},
		prioEnvironment: "prio-env",
		csrFQDN:         "some.fqdn",
		batchRotationCFG: key.RotationConfig{
			CreateKeyFunc:     key.P256.New,
			CreateMinAge:      10000 * time.Second,
			PrimaryMinAge:     1000 * time.Second,
			DeleteMinAge:      20000 * time.Second,
			DeleteMinKeyCount: 2,
		},
		packetRotationCFG: key.RotationConfig{
			CreateKeyFunc:     key.P256.New,
			CreateMinAge:      1000 * time.Second,
			PrimaryMinAge:     0,
			DeleteMinAge:      2000 * time.Second,
			DeleteMinKeyCount: 3,
		},
	}

	for _, test := range []struct {
		name string

		// Initial state.
		preBSKVersions  map[LI][]int64      // batch signing keys; (locality, ingestor) -> version timestamps; the first version is considered primary
		prePEKVersions  map[string][]int64  // packet encryption keys; locality -> version timestamps; the first version is considered primary
		preManifestInfo map[LI]manifestInfo // (locality, ingestor) -> manifest info

		// Desired state.
		postBSKVersions  map[LI][]int64      // batch signing keys; (locality, ingestor) -> version timestamps; the first version is considered primary
		postPEKVersions  map[string][]int64  // packet encryption keys; locality -> version timestamps; the first version is considered primary
		postManifestInfo map[LI]manifestInfo // (locality, ingestor) -> manifest info
	}{
		{
			name: "stable state",
			preBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {99600, 99000},
				li("asgard", "ingestor-2"): {99400, 99100},
			},
			prePEKVersions: map[string][]int64{
				"asgard": {99500},
			},
			preManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{99600, 99000},
					packetEncryptionKeyVersions: []int64{99500},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{99400, 99100},
					packetEncryptionKeyVersions: []int64{99500},
				},
			},

			postBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {99600, 99000},
				li("asgard", "ingestor-2"): {99400, 99100},
			},
			postPEKVersions: map[string][]int64{
				"asgard": {99500},
			},
			postManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{99600, 99000},
					packetEncryptionKeyVersions: []int64{99500},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{99400, 99100},
					packetEncryptionKeyVersions: []int64{99500},
				},
			},
		},

		{
			name: "first rotation for pre-rotation environment",
			preBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {0},
				li("asgard", "ingestor-2"): {0},
			},
			prePEKVersions: map[string][]int64{
				"asgard": {0},
			},
			preManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{0},
					packetEncryptionKeyVersions: []int64{0},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{0},
					packetEncryptionKeyVersions: []int64{0},
				},
			},

			postBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {100000, 0},
				li("asgard", "ingestor-2"): {100000, 0},
			},
			postPEKVersions: map[string][]int64{
				"asgard": {100000, 0},
			},
			postManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{100000, 0},
					packetEncryptionKeyVersions: []int64{100000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{100000, 0},
					packetEncryptionKeyVersions: []int64{100000},
				},
			},
		},

		{
			name: "first rotation for newly turned-up environment",
			preBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {},
				li("asgard", "ingestor-2"): {},
			},
			prePEKVersions: map[string][]int64{
				"asgard": {},
			},
			preManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{},
					packetEncryptionKeyVersions: []int64{},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{},
					packetEncryptionKeyVersions: []int64{},
				},
			},

			postBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {100000},
				li("asgard", "ingestor-2"): {100000},
			},
			postPEKVersions: map[string][]int64{
				"asgard": {100000},
			},
			postManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
			},
		},

		{
			name: "normal key rotation",
			preBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {50000},
				li("asgard", "ingestor-2"): {51000},
			},
			prePEKVersions: map[string][]int64{
				"asgard": {52000},
			},
			preManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{50000},
					packetEncryptionKeyVersions: []int64{52000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{51000},
					packetEncryptionKeyVersions: []int64{52000},
				},
			},

			postBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {50000, 100000},
				li("asgard", "ingestor-2"): {51000, 100000},
			},
			postPEKVersions: map[string][]int64{
				"asgard": {100000, 52000},
			},
			postManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{50000, 100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{51000, 100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
			},
		},

		{
			// this test starts as if "normal key rotation" ran previously, but
			// failed to write back the PEK & manifests
			name: "failure on previous run: key write failure",
			preBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {50000, 100000},
				li("asgard", "ingestor-2"): {51000, 100000},
			},
			prePEKVersions: map[string][]int64{
				"asgard": {52000},
			},
			preManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{50000},
					packetEncryptionKeyVersions: []int64{52000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{51000},
					packetEncryptionKeyVersions: []int64{52000},
				},
			},

			postBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {50000, 100000},
				li("asgard", "ingestor-2"): {51000, 100000},
			},
			postPEKVersions: map[string][]int64{
				"asgard": {100000, 52000},
			},
			postManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{50000, 100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{51000, 100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
			},
		},

		{
			// this test starts as if "normal key rotation" ran previously, but
			// failed to write back the asgard-ingestor-2 manifest
			name: "failure on previous run: manifest write failure",
			preBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {50000, 100000},
				li("asgard", "ingestor-2"): {51000, 100000},
			},
			prePEKVersions: map[string][]int64{
				"asgard": {100000, 52000},
			},
			preManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{50000, 100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{51000},
					packetEncryptionKeyVersions: []int64{52000},
				},
			},

			postBSKVersions: map[LI][]int64{
				li("asgard", "ingestor-1"): {50000, 100000},
				li("asgard", "ingestor-2"): {51000, 100000},
			},
			postPEKVersions: map[string][]int64{
				"asgard": {100000, 52000},
			},
			postManifestInfo: map[LI]manifestInfo{
				li("asgard", "ingestor-1"): {
					batchSigningKeyVersions:     []int64{50000, 100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
				li("asgard", "ingestor-2"): {
					batchSigningKeyVersions:     []int64{51000, 100000},
					packetEncryptionKeyVersions: []int64{100000},
				},
			},
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()

			// Cosntruct keys/manifests from initial key/manifest info, and
			// store them into key/manifest stores.
			keyStore := keyStore(test.preBSKVersions, test.prePEKVersions)
			manifestStore := manifestStore(test.preManifestInfo)

			preBSKs, prePEKs := dupLIToKeyMap(keyStore.BatchSigningKeys()), dupStrToKeyMap(keyStore.PacketEncryptionKeys())
			preManifests := dupStrToManifestMap(manifestStore.GetDataShareProcessorSpecificManifests())

			cfg := rotateKeysCFG
			cfg.keyStore, cfg.manifestStore = keyStore, manifestStore
			if err := rotateKeys(ctx, cfg); err != nil {
				t.Fatalf("Unexpected error from rotateKeys: %v", err)
			}

			// Verify batch signing keys.
			gotBSKs := keyStore.BatchSigningKeys()
			for li, gotK := range gotBSKs {
				gotVers := keyToVersionMap(gotK)

				// Verify versions match expected versions.
				wantVersLst, ok := test.postBSKVersions[li]
				if !ok {
					t.Errorf("Unexpected batch signing key for (%q, %q)", li.Locality, li.Ingestor)
					continue
				}
				wantVers := int64sToSet(wantVersLst)
				for wv := range wantVers {
					if _, ok := gotVers[wv]; !ok {
						t.Errorf("Batch signing key for (%q, %q) missing version %d", li.Locality, li.Ingestor, wv)
					}
				}
				for gv := range gotVers {
					if _, ok := wantVers[gv]; !ok {
						t.Errorf("Batch signing key for (%q, %q) has unexpected version %d", li.Locality, li.Ingestor, gv)
					}
				}

				// Verify that key versions that existed before rotation have the same key material.
				preVers := keyToVersionMap(preBSKs[li])
				for ts, gotVer := range gotVers {
					wantVer, ok := preVers[ts]
					if !ok {
						continue // this is a new version, nothing to compare back against
					}
					if !gotVer.KeyMaterial.Equal(wantVer.KeyMaterial) {
						t.Errorf("Batch signing key for (%q, %q) had unexpected key material change for version %d", li.Locality, li.Ingestor, ts)
					}
				}
			}
			for li := range test.postBSKVersions {
				if _, ok := gotBSKs[li]; !ok {
					t.Errorf("Missing expected batch signing key for (%q, %q)", li.Locality, li.Ingestor)
				}
			}

			// Verify packet encryption keys.
			gotPEKs := keyStore.PacketEncryptionKeys()
			for loc, gotK := range gotPEKs {
				gotVers := keyToVersionMap(gotK)

				// Verify versions match expected versions.
				wantVersLst, ok := test.postPEKVersions[loc]
				if !ok {
					t.Errorf("Unexpected packet encryption key for %q", loc)
					continue
				}
				wantVers := int64sToSet(wantVersLst)
				for wv := range wantVers {
					if _, ok := gotVers[wv]; !ok {
						t.Errorf("Packet encryption key for %q missing version %d", loc, wv)
					}
				}
				for gv := range gotVers {
					if _, ok := wantVers[gv]; !ok {
						t.Errorf("Packet encryption key for %q has unexpected version %d", loc, gv)
					}
				}

				// Verify that key versions that existed before rotation have the same key material.
				preVers := keyToVersionMap(prePEKs[loc])
				for ts, gotVer := range gotVers {
					wantVer, ok := preVers[ts]
					if !ok {
						continue // this is a new version, nothing to compare back against
					}
					if !gotVer.KeyMaterial.Equal(wantVer.KeyMaterial) {
						t.Errorf("Packet encryption key for %q has unexpected key material change for version %d", loc, ts)
					}
				}
			}
			for loc := range test.postPEKVersions {
				if _, ok := gotPEKs[loc]; !ok {
					t.Errorf("Missing expected packet encryption key for %q", loc)
				}
			}

			// Verify manifests.
			type wantManifestInfo struct {
				manifestInfo
				li LI
			}
			wantManifests := map[string]wantManifestInfo{}
			for li, info := range test.postManifestInfo {
				wantManifests[liToDSP(li)] = wantManifestInfo{info, li}
			}

			gotManifests := manifestStore.GetDataShareProcessorSpecificManifests()
			for dsp, gotM := range gotManifests {
				// Verify versions match expected versions.
				wantInfo, ok := wantManifests[dsp]
				if !ok {
					t.Errorf("Unexpected manifest for %q", dsp)
					continue
				}
				wantBSKVers := map[string]struct{}{}
				for _, ts := range wantInfo.batchSigningKeyVersions {
					wantBSKVers[bskKID(wantInfo.li, ts)] = struct{}{}
				}
				wantPEKVers := map[string]struct{}{}
				for _, ts := range wantInfo.packetEncryptionKeyVersions {
					wantPEKVers[pekKID(wantInfo.li.Locality, ts)] = struct{}{}
				}
				for wv := range wantBSKVers {
					if _, ok := gotM.BatchSigningPublicKeys[wv]; !ok {
						t.Errorf("Manifest for %q missing batch signing key version %q", dsp, wv)
					}
				}
				for gv := range gotM.BatchSigningPublicKeys {
					if _, ok := wantBSKVers[gv]; !ok {
						t.Errorf("Manifest for %q has unexpected batch signing key version %q", dsp, gv)
					}
				}
				for wv := range wantPEKVers {
					if _, ok := gotM.PacketEncryptionKeyCSRs[wv]; !ok {
						t.Errorf("Manifest for %q missing packet encryption key version %q", dsp, wv)
					}
				}
				for gv := range gotM.PacketEncryptionKeyCSRs {
					if _, ok := wantPEKVers[gv]; !ok {
						t.Errorf("Manifest for %q has unexpected packet encryption key version %q", dsp, gv)
					}
				}

				// Verify that key versions that existed before were copied without modification.
				preM := preManifests[dsp]
				for v, gotBSK := range gotM.BatchSigningPublicKeys {
					preBSK, ok := preM.BatchSigningPublicKeys[v]
					if !ok {
						continue // this is a new version, nothing to compare back against
					}
					if gotBSK != preBSK {
						t.Errorf("Manifest for %q has unexpected key material change for batch signing key %q", dsp, v)
					}
				}
				for v, gotPEK := range gotM.PacketEncryptionKeyCSRs {
					prePEK, ok := preM.PacketEncryptionKeyCSRs[v]
					if !ok {
						continue // this is a new version, nothing to compare back against
					}
					if gotPEK != prePEK {
						t.Errorf("Manifest for %q has unexpected key material change for packet encryption key %q", dsp, v)
					}
				}
			}
			for dsp := range wantManifests {
				if _, ok := gotManifests[dsp]; !ok {
					t.Errorf("Missing expected manifest for %q", dsp)
				}
			}
		})
	}
}

// keyStore creates a keystore with the given batch signing/packet encryption
// key versions, specified as a map from (locality, ingestor) or locality
// (respectively) to versions identified by UNIX second timestamps.
func keyStore(bskVersions map[LI][]int64, pekVersions map[string][]int64) *storagetest.Key {
	ks := storagetest.NewKey()

	bsks := ks.BatchSigningKeys()
	for li, vers := range bskVersions {
		bsks[li] = bsk(li, vers...)
	}

	peks := ks.PacketEncryptionKeys()
	for loc, vers := range pekVersions {
		peks[loc] = pek(loc, vers...)
	}

	return ks
}

// manifestStore creates a manifest store with the given manifests, specified
// as a map from data-share processor (i.e. locality & ingestor) to
// manifestInfo.
func manifestStore(manifestInfos map[LI]manifestInfo) *storagetest.Manifest {
	m := storagetest.NewManifest()
	ms := m.GetDataShareProcessorSpecificManifests()
	for li, info := range manifestInfos {
		bsks := manifest.BatchSigningPublicKeys{}
		for _, ts := range info.batchSigningKeyVersions {
			kid := bskKID(li, ts)
			pkix, err := keytest.Material(kid).PublicAsPKIX()
			if err != nil {
				panic(fmt.Sprintf("Couldn't serialize key material as PKIX: %v", err))
			}
			bsks[kid] = manifest.BatchSigningPublicKey{PublicKey: pkix}
		}
		peks := manifest.PacketEncryptionKeyCSRs{}
		for _, ts := range info.packetEncryptionKeyVersions {
			kid := pekKID(li.Locality, ts)
			csr, err := keytest.Material(kid).PublicAsCSR("some.fqdn")
			if err != nil {
				panic(fmt.Sprintf("Couldn't serialize key material as CSR: %v", err))
			}
			peks[kid] = manifest.PacketEncryptionCertificate{CertificateSigningRequest: csr}
		}
		ms[liToDSP(li)] = manifest.DataShareProcessorSpecificManifest{
			Format:                  1,
			IngestionIdentity:       "ingestion-identity",
			IngestionBucket:         "ingestion-bucket",
			PeerValidationIdentity:  "peer-validation-identity",
			PeerValidationBucket:    "peer-validation-bucket",
			BatchSigningPublicKeys:  bsks,
			PacketEncryptionKeyCSRs: peks,
		}
	}
	return m
}

// bsk creates a batch signing key with the given timestamps. Key material is
// arbitrary, but will match that of other batch signing keys at the same
// timestamp, locality, & ingestor, and will very likely not match other
// key materials. If timestamps are provided, the first timestamp is used as
// the primary key version.
func bsk(li LI, tss ...int64) key.Key {
	if len(tss) == 0 {
		return key.Key{}
	}
	var vs []key.Version
	for _, ts := range tss {
		vs = append(vs, key.Version{
			KeyMaterial:       keytest.Material(bskKID(li, ts)),
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
// same timestamp & locality, and will very likely not match other key
// materials. If timestamps are provided, the first timestamp is used as the
// primary key version.
func pek(locality string, tss ...int64) key.Key {
	if len(tss) == 0 {
		return key.Key{}
	}
	var vs []key.Version
	for _, ts := range tss {
		vs = append(vs, key.Version{
			KeyMaterial:       keytest.Material(pekKID(locality, ts)),
			CreationTimestamp: ts,
		})
	}
	k, err := key.FromVersions(vs[0], vs[1:]...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key: %v", err))
	}
	return k
}

func bskKID(li LI, ts int64) string {
	if ts == 0 {
		return fmt.Sprintf("prio-env-%s-%s-batch-signing-key", li.Locality, li.Ingestor)
	}
	return fmt.Sprintf("prio-env-%s-%s-batch-signing-key-%d", li.Locality, li.Ingestor, ts)
}

func pekKID(locality string, ts int64) string {
	if ts == 0 {
		return fmt.Sprintf("prio-env-%s-ingestion-packet-decryption-key", locality)
	}
	return fmt.Sprintf("prio-env-%s-ingestion-packet-decryption-key-%d", locality, ts)
}

func liToDSP(li LI) string { return fmt.Sprintf("%s-%s", li.Locality, li.Ingestor) }

// int64sToSet converts a slice of int64 to an equivalent set.
func int64sToSet(vals []int64) map[int64]struct{} {
	rslt := map[int64]struct{}{}
	for _, i := range vals {
		rslt[i] = struct{}{}
	}
	return rslt
}

// keyToVersionMap returns a map from UNIX second creation timestamps to key.
// versions.
func keyToVersionMap(k key.Key) map[int64]key.Version {
	rslt := map[int64]key.Version{}
	_ = k.Versions(func(v key.Version) error {
		rslt[v.CreationTimestamp] = v
		return nil
	})
	return rslt
}

// dupStrToKeyMap duplicates a map of strings to key.Keys.
func dupStrToKeyMap(m map[string]key.Key) map[string]key.Key {
	rslt := map[string]key.Key{}
	for k, v := range m {
		rslt[k] = v
	}
	return rslt
}

// dupLIToKeyMap duplicates a map of LIs to key.Keys.
func dupLIToKeyMap(m map[LI]key.Key) map[LI]key.Key {
	rslt := map[LI]key.Key{}
	for k, v := range m {
		rslt[k] = v
	}
	return rslt
}

// dupStrToManifestMap duplicates a map of strings to manifests.
func dupStrToManifestMap(m map[string]manifest.DataShareProcessorSpecificManifest) map[string]manifest.DataShareProcessorSpecificManifest {
	rslt := map[string]manifest.DataShareProcessorSpecificManifest{}
	for k, v := range m {
		rslt[k] = v
	}
	return rslt
}

func li(locality, ingestor string) LI { return LI{Locality: locality, Ingestor: ingestor} }
