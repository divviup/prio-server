package storage

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"errors"
	"fmt"
	"math/big"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"
	k8sapi "k8s.io/api/core/v1"
	k8smeta "k8s.io/apimachinery/pkg/apis/meta/v1"
	k8s "k8s.io/client-go/kubernetes/typed/core/v1"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

func TestKubernetesKey(t *testing.T) {
	t.Parallel()
	const (
		env      = "$ENV"
		locality = "$LOCALITY"
		ingestor = "$INGESTOR"
	)

	t.Run("BatchSigning", func(t *testing.T) {
		t.Parallel()

		// wantSecretKey taken directly from a dev environment secret
		// store. Other values derived from wantSecretKey.
		wantKey := k(pkv(0, mustP256From(&ecdsa.PrivateKey{
			PublicKey: ecdsa.PublicKey{
				Curve: elliptic.P256(),
				X:     mustInt("100281053943626114588339627807397740475849787919368479671799651521728988695054"),
				Y:     mustInt("92848018789799398563224167584887395252439620813688048638482994377853029146245"),
			},
			D: mustInt("8496960630434574126270397013403207859297604121831246711994989434547040199290"),
		})))
		const wantSecretKey = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgEskb+lNYa0/cmNi0uObi2XwdoZoJ5sDnIm2qBb98onqhRANCAATdtRCs2eUElaxYSPVjx0T90DuNQd5kCq2WFE9Q+U3KDs1GHce+HnELAbTFPmK10naqnRZw6FXfn5l9Aph7WV6F" // taken from a dev environment's actual secret
		const wantKeyVersions = `[{"key":"AQPdtRCs2eUElaxYSPVjx0T90DuNQd5kCq2WFE9Q+U3KDhLJG/pTWGtP3JjYtLjm4tl8HaGaCebA5yJtqgW/fKJ6","creation_time":"0","primary":true}]`

		t.Run("Put", func(t *testing.T) {
			t.Parallel()
			wantSD := map[string][]byte{"secret_key": []byte(wantSecretKey), "key_versions": []byte(wantKeyVersions)}
			store, sd := newK8sKey(env)
			putEmpty(sd, "$ENV-$LOCALITY-$INGESTOR-batch-signing-key")
			if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
			}
			gotSD := sd["$ENV-$LOCALITY-$INGESTOR-batch-signing-key"]
			if diff := cmp.Diff(wantSD, gotSD); diff != "" {
				t.Errorf("Batch signing key secret data differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			t.Run("FromSecretKey", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				putSecretKey(sd, "$ENV-$LOCALITY-$INGESTOR-batch-signing-key", []byte(wantSecretKey))
				gotKey, err := store.GetBatchSigningKey(ctx, locality, ingestor)
				if err != nil {
					t.Fatalf("Unexpected error from GetBatchSigningKey: %v", err)
				}
				if !wantKey.Equal(gotKey) {
					diff := cmp.Diff(wantKey, gotKey)
					t.Errorf("Key differs from expected (-want +got):\n%s", diff)
				}
			})
			t.Run("FromKeyVersions", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				putKeyVersions(sd, "$ENV-$LOCALITY-$INGESTOR-batch-signing-key", []byte(wantKeyVersions))
				gotKey, err := store.GetBatchSigningKey(ctx, locality, ingestor)
				if err != nil {
					t.Fatalf("Unexpected error from GetBatchSigningKey: %v", err)
				}
				if !wantKey.Equal(gotKey) {
					diff := cmp.Diff(wantKey, gotKey)
					t.Errorf("Key differs from expected (-want +got):\n%s", diff)
				}
			})
			t.Run("Empty", func(t *testing.T) {
				t.Parallel()
				var wantKey key.Key // empty key
				store, sd := newK8sKey(env)
				putEmpty(sd, "$ENV-$LOCALITY-$INGESTOR-batch-signing-key")
				gotKey, err := store.GetBatchSigningKey(ctx, locality, ingestor)
				if err != nil {
					t.Fatalf("Unexpected error from GetBatchSigningKey: %v", err)
				}
				if !wantKey.Equal(gotKey) {
					diff := cmp.Diff(wantKey, gotKey)
					t.Errorf("Key differs from expected (-want +got):\n%s", diff)
				}
			})
		})

		t.Run("RoundTrip", func(t *testing.T) {
			t.Parallel()
			store, sd := newK8sKey(env)
			putEmpty(sd, "$ENV-$LOCALITY-$INGESTOR-batch-signing-key")
			if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
			}
			gotKey, err := store.GetBatchSigningKey(ctx, locality, ingestor)
			if err != nil {
				t.Fatalf("Unexpected error from GetBatchSigningKey: %v", err)
			}
			if !wantKey.Equal(gotKey) {
				diff := cmp.Diff(wantKey, gotKey)
				t.Errorf("Key differs from expected (-want +got):\n%s", diff)
			}
		})
	})

	t.Run("PacketEncryption", func(t *testing.T) {
		t.Parallel()

		// wantSecretKey taken directly from a dev environment secret
		// store. Other values derived from wantSecretKey.
		wantKey := k(pkv(0, mustP256From(&ecdsa.PrivateKey{
			PublicKey: ecdsa.PublicKey{
				Curve: elliptic.P256(),
				X:     mustInt("78527022544260903523204947018872622072202784880351210249668611210032537819764"),
				Y:     mustInt("22745617558975184728664387250484621262807351942545566697101728810261708479900"),
			},
			D: mustInt("7417359065569682521889946159093475243201835077729681597084399613736246477746929664"),
		})))
		const wantSecretKey = "BK2cuD4p1h6OEMMwaBh1UfJq7PAK8HgnQ/ztl3PFIlp0MkmQNYJkekvodLyqcte2t3WQoejx0J9/QqhZ19/fHZz6OZAo45m1iEaeq0f20adSOgf53w/5jvPwlLE/Tss3EQAA" // taken from a dev environment's actual secret
		const wantKeyVersions = `[{"key":"AQKtnLg+KdYejhDDMGgYdVHyauzwCvB4J0P87ZdzxSJadPo5kCjjmbWIRp6rR/bRp1I6B/nfD/mO8/CUsT9OyzcRAAA","creation_time":"0","primary":true}]`

		t.Run("Put", func(t *testing.T) {
			t.Parallel()
			wantSD := map[string][]byte{"secret_key": []byte(wantSecretKey), "key_versions": []byte(wantKeyVersions)}
			store, sd := newK8sKey(env)
			putEmpty(sd, "$ENV-$LOCALITY-ingestion-packet-decryption-key")
			if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
			}
			gotSD := sd["$ENV-$LOCALITY-ingestion-packet-decryption-key"]
			if diff := cmp.Diff(wantSD, gotSD); diff != "" {
				t.Errorf("Packet encryption key secret data differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			t.Run("FromSecretKey", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				putSecretKey(sd, "$ENV-$LOCALITY-ingestion-packet-decryption-key", []byte(wantSecretKey))
				gotKey, err := store.GetPacketEncryptionKey(ctx, locality)
				if err != nil {
					t.Fatalf("Unexpected error from GetPacketEncryptionKey: %v", err)
				}
				if !wantKey.Equal(gotKey) {
					diff := cmp.Diff(wantKey, gotKey)
					t.Errorf("Key differs from expected (-want +got):\n%s", diff)
				}
			})
			t.Run("FromKeyVersions", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				putKeyVersions(sd, "$ENV-$LOCALITY-ingestion-packet-decryption-key", []byte(wantKeyVersions))
				gotKey, err := store.GetPacketEncryptionKey(ctx, locality)
				if err != nil {
					t.Fatalf("Unexpected error from GetPacketEncryptionKey: %v", err)
				}
				if !wantKey.Equal(gotKey) {
					diff := cmp.Diff(wantKey, gotKey)
					t.Errorf("Key differs from expected (-want +got):\n%s", diff)
				}
			})
			t.Run("Empty", func(t *testing.T) {
				t.Parallel()
				var wantKey key.Key // empty key
				store, sd := newK8sKey(env)
				putEmpty(sd, "$ENV-$LOCALITY-ingestion-packet-decryption-key")
				gotKey, err := store.GetPacketEncryptionKey(ctx, locality)
				if err != nil {
					t.Fatalf("Unexpected error from GetPacketEncryptionKey: %v", err)
				}
				if !wantKey.Equal(gotKey) {
					diff := cmp.Diff(wantKey, gotKey)
					t.Errorf("Key differs from expected (-want +got):\n%s", diff)
				}
			})
		})

		t.Run("RoundTrip", func(t *testing.T) {
			t.Parallel()
			store, sd := newK8sKey(env)
			putEmpty(sd, "$ENV-$LOCALITY-ingestion-packet-decryption-key")
			if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
			}
			gotKey, err := store.GetPacketEncryptionKey(ctx, locality)
			if err != nil {
				t.Fatalf("Unexpected error from GetPacketEncryptionKey: %v", err)
			}
			if !wantKey.Equal(gotKey) {
				diff := cmp.Diff(wantKey, gotKey)
				t.Errorf("Key differs from expected (-want +got):\n%s", diff)
			}
		})
	})
}

func mustP256From(privKey *ecdsa.PrivateKey) key.Material {
	k, err := key.P256MaterialFrom(privKey)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create new P256 key: %v", err))
	}
	return k
}

// k creates a new key or dies trying.
func k(vs ...key.Version) key.Key {
	k, err := key.FromVersions(vs...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key from versions: %v", err))
	}
	return k
}

// kv creates a non-primary key version with the given timestamp and raw key.
func kv(ts int64, k key.Material) key.Version {
	return key.Version{
		KeyMaterial:  k,
		CreationTime: time.Unix(ts, 0),
	}
}

// pkv creates a primary key version with the given timestamp and raw key.
func pkv(ts int64, k key.Material) key.Version {
	kv := kv(ts, k)
	kv.Primary = true
	return kv
}

func putEmpty(sd map[string]map[string][]byte, name string) {
	sd[name] = map[string][]byte{"secret_key": []byte("not-a-real-key")}
}

func putSecretKey(sd map[string]map[string][]byte, name string, value []byte) {
	sd[name] = map[string][]byte{"secret_key": value}
}

func putKeyVersions(sd map[string]map[string][]byte, name string, value []byte) {
	sd[name] = map[string][]byte{"key_versions": value}
}

func mustInt(digits string) *big.Int {
	var z big.Int
	if _, ok := z.SetString(digits, 10); !ok {
		panic(fmt.Sprintf("Couldn't set digits of big.Int to %q", digits))
	}
	return &z
}

// newK8sKey creates a new Kubernetes-based key implementation, based on a
// Kubernetes fake that reads & writes secrets data from the returned map.
func newK8sKey(env string) (Key, map[string]map[string][]byte) {
	sd := map[string]map[string][]byte{}
	return k8sKey{fakeK8sSecret{sd: sd}, env}, sd
}

type fakeK8sSecret struct {
	k8s.SecretInterface
	sd map[string]map[string][]byte
}

func (s fakeK8sSecret) Get(_ context.Context, name string, _ k8smeta.GetOptions) (*k8sapi.Secret, error) {
	sd, ok := s.sd[name]
	if !ok {
		return nil, fmt.Errorf("no such key %q", name)
	}
	secret := &k8sapi.Secret{
		ObjectMeta: k8smeta.ObjectMeta{Name: name},
		Data:       map[string][]byte{},
	}
	for k, v := range sd {
		vCopy := make([]byte, len(v))
		copy(vCopy, v)
		secret.Data[k] = vCopy
	}
	return secret, nil
}

func (s fakeK8sSecret) Update(_ context.Context, secret *k8sapi.Secret, _ k8smeta.UpdateOptions) (*k8sapi.Secret, error) {
	name := secret.ObjectMeta.Name
	if name == "" {
		return nil, errors.New("missing name")
	}
	sd := map[string][]byte{}
	for k, v := range secret.Data {
		vCopy := make([]byte, len(v))
		copy(vCopy, v)
		sd[k] = vCopy
	}
	s.sd[name] = sd
	return secret, nil
}
