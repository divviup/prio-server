package storage

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"errors"
	"fmt"
	"math/big"
	"strings"
	"testing"

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
		const secretName = "$ENV-$LOCALITY-$INGESTOR-batch-signing-key"
		wantKey := k(kv(0, mustP256From(&ecdsa.PrivateKey{
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
			wantSD := map[string][]byte{"secret_key": []byte(wantSecretKey), "key_versions": []byte(wantKeyVersions), "primary_kid": []byte(secretName)}
			store, sd := newK8sKey(env)
			putEmpty(sd, secretName)
			if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
			}
			gotSD := sd[secretName]
			if diff := cmp.Diff(wantSD, gotSD); diff != "" {
				t.Errorf("Batch signing key secret data differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			t.Run("FromSecretKey", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				putSecretKey(sd, secretName, []byte(wantSecretKey))
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
				putKeyVersions(sd, secretName, []byte(wantKeyVersions))
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
				putEmpty(sd, secretName)
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
			putEmpty(sd, secretName)
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
		const secretName = "$ENV-$LOCALITY-ingestion-packet-decryption-key"
		wantKey := k(kv(0, mustP256From(&ecdsa.PrivateKey{
			PublicKey: ecdsa.PublicKey{
				Curve: elliptic.P256(),
				X:     mustInt("30176607170335032169658242164164128826835373272126183526750106092296931195595"),
				Y:     mustInt("108357461753906582253786936365593814628895529785013716760654244774460420655478"),
			},
			D: mustInt("79147076814969273829273941581782929461090592164040248130581241256142186071590"),
		})))
		const wantSecretKey = "BEK3Wrk7GOIkNmFqgbSN/P0eDFexXgSLWm7SrcPitTrL75AmZBrL3IoNy4CdrFJt5br2jA0RSkrnFLh5FAvuCXau+6hxT2U6N4c8sTbUJIRQe25MQ1peZQ4J0FZuChXGJg==" // taken from a dev environment's actual secret
		const wantKeyVersions = `[{"key":"AQJCt1q5OxjiJDZhaoG0jfz9HgxXsV4Ei1pu0q3D4rU6y677qHFPZTo3hzyxNtQkhFB7bkxDWl5lDgnQVm4KFcYm","creation_time":"0","primary":true}]`

		t.Run("Put", func(t *testing.T) {
			t.Parallel()
			wantSD := map[string][]byte{"secret_key": []byte(wantSecretKey), "key_versions": []byte(wantKeyVersions), "primary_kid": []byte(secretName)}
			store, sd := newK8sKey(env)
			putEmpty(sd, secretName)
			if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
			}
			gotSD := sd[secretName]
			if diff := cmp.Diff(wantSD, gotSD); diff != "" {
				t.Errorf("Packet encryption key secret data differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			t.Run("FromSecretKey", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				putSecretKey(sd, secretName, []byte(wantSecretKey))
				gotKey, err := store.GetPacketEncryptionKey(ctx, locality)
				if err != nil {
					t.Fatalf("Unexpected error from GetPacketEncryptionKey: %v", err)
				}
				if !wantKey.Equal(gotKey) {
					diff := cmp.Diff(wantKey, gotKey)
					t.Errorf("Key differs from expected (-want +got):\n%s", diff)
				}
			})
			t.Run("FromSecretKey: wrong length", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				badSecretKey := wantSecretKey[:len(wantSecretKey)-4] // shave off a few bytes
				putSecretKey(sd, secretName, []byte(badSecretKey))
				const wantErrStr = "key was wrong length"
				if _, err := store.GetPacketEncryptionKey(ctx, locality); err == nil || !strings.Contains(err.Error(), wantErrStr) {
					t.Errorf("Wanted error from GetPacketEncryptionKey containing %q, got: %v", wantErrStr, err)
				}
			})
			t.Run("FromKeyVersions", func(t *testing.T) {
				t.Parallel()
				store, sd := newK8sKey(env)
				putKeyVersions(sd, secretName, []byte(wantKeyVersions))
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
				putEmpty(sd, secretName)
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
			putEmpty(sd, secretName)
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

// k creates a new key or dies trying. pkv is the primary key version, vs are
// other versions.
func k(pkv key.Version, vs ...key.Version) key.Key {
	k, err := key.FromVersions(pkv, vs...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key from versions: %v", err))
	}
	return k
}

// kv creates a non-primary key version with the given timestamp and raw key.
func kv(ts int64, k key.Material) key.Version {
	return key.Version{
		KeyMaterial:       k,
		CreationTimestamp: ts,
	}
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
