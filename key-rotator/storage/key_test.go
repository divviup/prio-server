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

	"github.com/aws/aws-sdk-go/aws/awserr"
	"github.com/aws/aws-sdk-go/aws/request"
	"github.com/aws/aws-sdk-go/service/secretsmanager"
	"github.com/google/go-cmp/cmp"
	"github.com/googleapis/gax-go/v2"
	smpb "google.golang.org/genproto/googleapis/cloud/secretmanager/v1"
	k8sapi "k8s.io/api/core/v1"
	k8smeta "k8s.io/apimachinery/pkg/apis/meta/v1"
	k8s "k8s.io/client-go/kubernetes/typed/core/v1"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

const (
	env          = "$ENV"
	gcpProjectID = "$GCP_PROJECT_ID"
	locality     = "$LOCALITY"
	ingestor     = "$INGESTOR"

	bskSecretName = "$ENV-$LOCALITY-$INGESTOR-batch-signing-key"
	pekSecretName = "$ENV-$LOCALITY-ingestion-packet-decryption-key"

	// wantBSKSecretKey taken directly from a dev environment secret.
	// wantPEKSecretKey, wantKeyVersions, wantKey derived from wantBSKSecretKey.
	wantBSKSecretKey = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgEskb+lNYa0/cmNi0uObi2XwdoZoJ5sDnIm2qBb98onqhRANCAATdtRCs2eUElaxYSPVjx0T90DuNQd5kCq2WFE9Q+U3KDs1GHce+HnELAbTFPmK10naqnRZw6FXfn5l9Aph7WV6F" // taken from a dev environment's actual secret
	wantPEKSecretKey = "BN21EKzZ5QSVrFhI9WPHRP3QO41B3mQKrZYUT1D5TcoOzUYdx74ecQsBtMU+YrXSdqqdFnDoVd+fmX0CmHtZXoUSyRv6U1hrT9yY2LS45uLZfB2hmgnmwOcibaoFv3yieg=="
	wantKeyVersions  = `[{"key":"AQPdtRCs2eUElaxYSPVjx0T90DuNQd5kCq2WFE9Q+U3KDhLJG/pTWGtP3JjYtLjm4tl8HaGaCebA5yJtqgW/fKJ6","creation_time":"0","primary":true}]`
)

var (
	wantKey = k(kv(0, mustP256From(&ecdsa.PrivateKey{
		PublicKey: ecdsa.PublicKey{
			Curve: elliptic.P256(),
			X:     mustInt("100281053943626114588339627807397740475849787919368479671799651521728988695054"),
			Y:     mustInt("92848018789799398563224167584887395252439620813688048638482994377853029146245"),
		},
		D: mustInt("8496960630434574126270397013403207859297604121831246711994989434547040199290"),
	})))
)

func TestKubernetesKey(t *testing.T) {
	t.Parallel()

	t.Run("BatchSigning", func(t *testing.T) {
		t.Parallel()

		t.Run("Put", func(t *testing.T) {
			t.Parallel()
			wantSD := map[string][]byte{"secret_key": []byte(wantBSKSecretKey), "key_versions": []byte(wantKeyVersions), "primary_kid": []byte(bskSecretName)}
			store, k8s := newK8sKey()
			k8s.putEmpty(bskSecretName)
			if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
			}
			gotSD := k8s.sd[bskSecretName]
			if diff := cmp.Diff(wantSD, gotSD); diff != "" {
				t.Errorf("Batch signing key secret data differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			t.Run("FromSecretKey", func(t *testing.T) {
				t.Parallel()
				store, k8s := newK8sKey()
				k8s.putSecretKey(bskSecretName, []byte(wantBSKSecretKey))
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
				store, k8s := newK8sKey()
				k8s.putKeyVersions(bskSecretName, []byte(wantKeyVersions))
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
				store, k8s := newK8sKey()
				k8s.putEmpty(bskSecretName)
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
			store, k8s := newK8sKey()
			k8s.putEmpty(bskSecretName)
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

		t.Run("Put", func(t *testing.T) {
			t.Parallel()
			wantSD := map[string][]byte{"secret_key": []byte(wantPEKSecretKey), "key_versions": []byte(wantKeyVersions), "primary_kid": []byte(pekSecretName)}
			store, k8s := newK8sKey()
			k8s.putEmpty(pekSecretName)
			if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
				t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
			}
			gotSD := k8s.sd[pekSecretName]
			if diff := cmp.Diff(wantSD, gotSD); diff != "" {
				t.Errorf("Packet encryption key secret data differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			t.Run("FromSecretKey", func(t *testing.T) {
				t.Parallel()
				store, k8s := newK8sKey()
				k8s.putSecretKey(pekSecretName, []byte(wantPEKSecretKey))
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
				store, k8s := newK8sKey()
				k8s.putKeyVersions(pekSecretName, []byte(wantKeyVersions))
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
				store, k8s := newK8sKey()
				k8s.putEmpty(pekSecretName)
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
			store, k8s := newK8sKey()
			k8s.putEmpty(pekSecretName)
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

func TestAWSKey(t *testing.T) {
	t.Parallel()

	t.Run("BatchSigning", func(t *testing.T) {
		t.Parallel()

		t.Run("Put", func(t *testing.T) {
			t.Parallel()

			t.Run("key already exists", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, aws := newAWSKey()
				aws.put(bskSecretName, []byte("arbitrary existing key material"))
				if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
				}
				gotSD := aws.sd[bskSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Batch signing key secret data differs from expected (-want +got):\n%s", diff)
				}
			})

			t.Run("key does not already exist", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, aws := newAWSKey()
				if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
				}
				gotSD := aws.sd[bskSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Batch signing key secret data differs from expected (-want +got):\n%s", diff)
				}
			})
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			store, aws := newAWSKey()
			aws.put(bskSecretName, []byte(wantKeyVersions))
			gotKey, err := store.GetBatchSigningKey(ctx, locality, ingestor)
			if err != nil {
				t.Fatalf("Unexpected error from GetBatchSigningKey: %v", err)
			}
			if !wantKey.Equal(gotKey) {
				diff := cmp.Diff(wantKey, gotKey)
				t.Errorf("Key differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("RoundTrip", func(t *testing.T) {
			t.Parallel()
			store, _ := newAWSKey()
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

		t.Run("Put", func(t *testing.T) {
			t.Parallel()

			t.Run("key already exists", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, aws := newAWSKey()
				aws.put(bskSecretName, []byte("arbitrary existing key material"))
				if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
				}
				gotSD := aws.sd[pekSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Packet encryption key secret data differs from expected (-want +got):\n%s", diff)
				}
			})

			t.Run("key does not already exist", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, aws := newAWSKey()
				if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
				}
				gotSD := aws.sd[pekSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Packet encryption key secret data differs from expected (-want +got):\n%s", diff)
				}
			})
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			store, aws := newAWSKey()
			aws.put(pekSecretName, []byte(wantKeyVersions))
			gotKey, err := store.GetPacketEncryptionKey(ctx, locality)
			if err != nil {
				t.Fatalf("Unexpected error from GetPacketEncryptionKey: %v", err)
			}
			if !wantKey.Equal(gotKey) {
				diff := cmp.Diff(wantKey, gotKey)
				t.Errorf("Key differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("RoundTrip", func(t *testing.T) {
			t.Parallel()
			store, _ := newAWSKey()
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

func TestGCPKey(t *testing.T) {
	t.Parallel()

	t.Run("BatchSigning", func(t *testing.T) {
		t.Parallel()

		t.Run("Put", func(t *testing.T) {
			t.Parallel()

			t.Run("key already exists", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, gcp := newGCPKey()
				gcp.put(bskSecretName, []byte("arbitrary existing key material"))
				if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
				}
				gotSD := gcp.sd[bskSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Batch signing key secret data differs from expected (-want +got):\n%s", diff)
				}
			})

			t.Run("key does not already exist", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, gcp := newGCPKey()
				if err := store.PutBatchSigningKey(ctx, locality, ingestor, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutBatchSigningKey: %v", err)
				}
				gotSD := gcp.sd[bskSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Batch signing key secret data differs from expected (-want +got):\n%s", diff)
				}
			})
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			store, gcp := newGCPKey()
			gcp.put(bskSecretName, []byte(wantKeyVersions))
			gotKey, err := store.GetBatchSigningKey(ctx, locality, ingestor)
			if err != nil {
				t.Fatalf("Unexpected error from GetBatchSigningKey: %v", err)
			}
			if !wantKey.Equal(gotKey) {
				diff := cmp.Diff(wantKey, gotKey)
				t.Errorf("Key differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("RoundTrip", func(t *testing.T) {
			t.Parallel()
			store, _ := newGCPKey()
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

		t.Run("Put", func(t *testing.T) {
			t.Parallel()

			t.Run("key already exists", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, gcp := newGCPKey()
				gcp.put(bskSecretName, []byte("arbitrary existing key material"))
				if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
				}
				gotSD := gcp.sd[pekSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Packet encryption key secret data differs from expected (-want +got):\n%s", diff)
				}
			})

			t.Run("key does not already exist", func(t *testing.T) {
				t.Parallel()
				wantSD := []byte(wantKeyVersions)
				store, gcp := newGCPKey()
				if err := store.PutPacketEncryptionKey(ctx, locality, wantKey); err != nil {
					t.Fatalf("Unexpected error from PutPacketEncryptionKey: %v", err)
				}
				gotSD := gcp.sd[pekSecretName]
				if diff := cmp.Diff(wantSD, gotSD); diff != "" {
					t.Errorf("Packet encryption key secret data differs from expected (-want +got):\n%s", diff)
				}
			})
		})

		t.Run("Get", func(t *testing.T) {
			t.Parallel()
			store, gcp := newGCPKey()
			gcp.put(pekSecretName, []byte(wantKeyVersions))
			gotKey, err := store.GetPacketEncryptionKey(ctx, locality)
			if err != nil {
				t.Fatalf("Unexpected error from GetPacketEncryptionKey: %v", err)
			}
			if !wantKey.Equal(gotKey) {
				diff := cmp.Diff(wantKey, gotKey)
				t.Errorf("Key differs from expected (-want +got):\n%s", diff)
			}
		})

		t.Run("RoundTrip", func(t *testing.T) {
			t.Parallel()
			store, _ := newGCPKey()
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

func mustInt(digits string) *big.Int {
	var z big.Int
	if _, ok := z.SetString(digits, 10); !ok {
		panic(fmt.Sprintf("Couldn't set digits of big.Int to %q", digits))
	}
	return &z
}

// newK8sKey creates a new Kubernetes-based key implementation, based on a
// Kubernetes fake that reads & writes secrets data to memory.
func newK8sKey() (Key, fakeK8sSecret) {
	k8s := fakeK8sSecret{sd: map[string]map[string][]byte{}}
	return k8sKey{k8s, env}, k8s
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

func (s fakeK8sSecret) putEmpty(name string) {
	s.sd[name] = map[string][]byte{"secret_key": []byte("not-a-real-key")}
}

func (s fakeK8sSecret) putSecretKey(name string, value []byte) {
	s.sd[name] = map[string][]byte{"secret_key": value}
}

func (s fakeK8sSecret) putKeyVersions(name string, value []byte) {
	s.sd[name] = map[string][]byte{"key_versions": value}
}

func newAWSKey() (Key, fakeAWSSecretManager) {
	aws := fakeAWSSecretManager{sd: map[string][]byte{}}
	return awsKey{aws, env}, aws
}

type fakeAWSSecretManager struct{ sd map[string][]byte }

func (m fakeAWSSecretManager) CreateSecretWithContext(_ context.Context, req *secretsmanager.CreateSecretInput, _ ...request.Option) (*secretsmanager.CreateSecretOutput, error) {
	if req.Name == nil {
		return nil, errors.New("Name is nil")
	}
	secretName := *req.Name
	if _, ok := m.sd[secretName]; ok {
		return nil, awserr.New(secretsmanager.ErrCodeResourceExistsException, fmt.Sprintf("secret %q already exists", secretName), nil)
	}
	m.sd[secretName] = nil
	return nil, nil
}

func (m fakeAWSSecretManager) GetSecretValueWithContext(_ context.Context, req *secretsmanager.GetSecretValueInput, _ ...request.Option) (*secretsmanager.GetSecretValueOutput, error) {
	if req.SecretId == nil {
		return nil, errors.New("SecretId is nil")
	}
	secretName := *req.SecretId
	secretBytes, ok := m.sd[secretName]
	switch {
	case !ok:
		return nil, fmt.Errorf("no such secret %q", secretName)
	case secretBytes == nil:
		return nil, fmt.Errorf("secret %q has no versions", secretName)
	}
	return &secretsmanager.GetSecretValueOutput{SecretBinary: secretBytes}, nil
}

func (m fakeAWSSecretManager) PutSecretValueWithContext(_ context.Context, req *secretsmanager.PutSecretValueInput, _ ...request.Option) (*secretsmanager.PutSecretValueOutput, error) {
	switch {
	case req.SecretId == nil:
		return nil, errors.New("SecretId is nil")
	case req.SecretBinary == nil:
		return nil, errors.New("SecretBinary is nil")
	}
	secretName := *req.SecretId
	if _, ok := m.sd[secretName]; !ok {
		return nil, fmt.Errorf("no such secret %q", secretName)
	}
	m.sd[secretName] = req.SecretBinary
	return nil, nil
}

func (m fakeAWSSecretManager) put(name string, value []byte) { m.sd[name] = value }

func newGCPKey() (Key, fakeGCPSecretManager) {
	gcp := fakeGCPSecretManager{sd: map[string][]byte{}}
	return gcpKey{gcp, env, gcpProjectID}, gcp
}

type fakeGCPSecretManager struct{ sd map[string][]byte }

func (m fakeGCPSecretManager) AccessSecretVersion(_ context.Context, req *smpb.AccessSecretVersionRequest, _ ...gax.CallOption) (*smpb.AccessSecretVersionResponse, error) {
	const (
		wantPrefix = "projects/" + gcpProjectID + "/secrets/"
		wantSuffix = "/versions/latest"
	)
	switch {
	case !strings.HasPrefix(req.Name, wantPrefix):
		return nil, fmt.Errorf("unexpected Name (got %q, want something prefixed with %q)", req.Name, wantPrefix)
	case !strings.HasSuffix(req.Name, wantSuffix):
		return nil, fmt.Errorf("unexpected Name (got %q, want something suffixed with %q)", req.Name, wantSuffix)
	}
	secretName := strings.TrimPrefix(strings.TrimSuffix(req.Name, wantSuffix), wantPrefix)
	secretBytes, ok := m.sd[secretName]
	switch {
	case !ok:
		return nil, fmt.Errorf("no such secret %q", secretName)
	case secretBytes == nil:
		return nil, fmt.Errorf("secret %q has no versions", secretName)
	}
	return &smpb.AccessSecretVersionResponse{Payload: &smpb.SecretPayload{Data: secretBytes}}, nil
}

func (m fakeGCPSecretManager) AddSecretVersion(_ context.Context, req *smpb.AddSecretVersionRequest, _ ...gax.CallOption) (*smpb.SecretVersion, error) {
	const wantPrefix = "projects/" + gcpProjectID + "/secrets/"
	if !strings.HasPrefix(req.Parent, wantPrefix) {
		return nil, fmt.Errorf("unexpected Parent (got %q, want something prefixed with %q)", req.Parent, wantPrefix)
	}
	secretName := strings.TrimPrefix(req.Parent, wantPrefix)
	if _, ok := m.sd[secretName]; !ok {
		return nil, fmt.Errorf("no such secret %q", secretName)
	}
	m.sd[secretName] = req.Payload.Data
	return nil, nil
}

func (m fakeGCPSecretManager) CreateSecret(_ context.Context, req *smpb.CreateSecretRequest, _ ...gax.CallOption) (*smpb.Secret, error) {
	const wantParent = "projects/" + gcpProjectID
	if req.Parent != wantParent {
		return nil, fmt.Errorf("unexpected Parent (got %q, want %q)", req.Parent, wantParent)
	}
	// XXX: if secret already exists, return an appropriate error instead of no-op (need to figure out correct error to return & update main-program logic first)
	if _, ok := m.sd[req.SecretId]; ok {
		return nil, nil
	}
	m.sd[req.SecretId] = nil
	return nil, nil
}

func (m fakeGCPSecretManager) put(name string, value []byte) { m.sd[name] = value }
