package storage

import (
	"context"
	"encoding/json"
	"errors"
	"path"
	"strings"
	"testing"

	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	"github.com/google/go-cmp/cmp"
)

var ctx = context.Background()

func TestManifest(t *testing.T) {
	t.Parallel()

	// Make up a data-share specific & global manifest. Contents are mostly arbitrary.
	const dspName = "dsp"
	dspManifest := manifest.DataShareProcessorSpecificManifest{
		Format:            12,
		IngestionIdentity: "ingestion_identity",
		IngestionBucket:   "ingestion_bucket",
	}
	dspManifestBytes, err := json.Marshal(dspManifest)
	if err != nil {
		t.Fatalf("Couldn't marshal data share processor-specific manifest to JSON: %v", err)
	}
	globalManifest := manifest.IngestorGlobalManifest{
		Format: 36,
		ServerIdentity: manifest.ServerIdentity{
			AWSIamEntity:           "aws_iam_entity",
			GCPServiceAccountEmail: "gcp_service_account_email",
		},
	}
	globalManifestBytes, err := json.Marshal(globalManifest)
	if err != nil {
		t.Fatalf("Couldn't marshal global manifest to JSON: %v", err)
	}

	for _, test := range []struct {
		name      string
		keyPrefix string
	}{
		{"no key prefix", ""},
		{"key prefix", "some/key/prefix"},
		{"directory-like key prefix", "some/dir/key/prefix/"},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Run("PutDataShareProcessorSpecificManifest", func(t *testing.T) {
				t.Parallel()
				m, kvs := newKVStoreManifest(test.keyPrefix)
				wantKVs := map[string][]byte{path.Join(test.keyPrefix, "dsp-manifest.json"): dspManifestBytes}
				if err := m.PutDataShareProcessorSpecificManifest(ctx, dspName, dspManifest); err != nil {
					t.Fatalf("Unexpected error from PutDataShareProcessorSpecificManifest: %v", err)
				}
				if diff := cmp.Diff(wantKVs, kvs); diff != "" {
					t.Errorf("Unexpected datastore content (-want +got):\n%s", diff)
				}
			})

			t.Run("PutIngestorGlobalManifest", func(t *testing.T) {
				t.Parallel()
				m, kvs := newKVStoreManifest(test.keyPrefix)
				wantKVs := map[string][]byte{path.Join(test.keyPrefix, "global-manifest.json"): globalManifestBytes}
				if err := m.PutIngestorGlobalManifest(ctx, globalManifest); err != nil {
					t.Fatalf("Unexpected error from PutIngestorGlobalManifest: %v", err)
				}
				if diff := cmp.Diff(wantKVs, kvs); diff != "" {
					t.Errorf("Unexpected datastore content (-want +got):\n%s", diff)
				}
			})

			t.Run("GetDataShareProcessorSpecificManifest", func(t *testing.T) {
				t.Parallel()
				t.Run("valid manifest", func(t *testing.T) {
					t.Parallel()
					m, kvs := newKVStoreManifest(test.keyPrefix)
					kvs[path.Join(test.keyPrefix, "dsp-manifest.json")] = dspManifestBytes
					gotManifest, err := m.GetDataShareProcessorSpecificManifest(ctx, dspName)
					if err != nil {
						t.Fatalf("Unexpected error from GetDataShareProcessorSpecificManifest: %v", err)
					}
					if diff := cmp.Diff(dspManifest, gotManifest); diff != "" {
						t.Errorf("Unexpected manifest (-want +got):\n%s", diff)
					}
				})

				t.Run("valid manifest (default specified)", func(t *testing.T) {
					t.Parallel()
					m, kvs := newKVStoreManifest(test.keyPrefix)
					m.defaultManifestByDSP = map[string]manifest.DataShareProcessorSpecificManifest{
						dspName: {Format: 13},
					}
					kvs[path.Join(test.keyPrefix, "dsp-manifest.json")] = dspManifestBytes
					gotManifest, err := m.GetDataShareProcessorSpecificManifest(ctx, dspName)
					if err != nil {
						t.Fatalf("Unexpected error from GetDataShareProcessorSpecificManifest: %v", err)
					}
					if diff := cmp.Diff(dspManifest, gotManifest); diff != "" {
						t.Errorf("Unexpected manifest (-want +got):\n%s", diff)
					}
				})

				t.Run("no manifest", func(t *testing.T) {
					t.Parallel()
					m, _ := newKVStoreManifest(test.keyPrefix)
					if _, err := m.GetDataShareProcessorSpecificManifest(ctx, dspName); !errors.Is(err, ErrObjectNotExist) {
						t.Errorf("Unexpected error from GetDataShareProcessorSpecificManifest: %v", err)
					}
				})

				t.Run("no manifest (default specified)", func(t *testing.T) {
					t.Parallel()
					m, _ := newKVStoreManifest(test.keyPrefix)
					wantManifest := manifest.DataShareProcessorSpecificManifest{Format: 13}
					m.defaultManifestByDSP = map[string]manifest.DataShareProcessorSpecificManifest{
						dspName: wantManifest,
					}
					gotManifest, err := m.GetDataShareProcessorSpecificManifest(ctx, dspName)
					if err != nil {
						t.Fatalf("Unexpected error from GetDataShareProcessorSpecificManifest: %v", err)
					}
					if diff := cmp.Diff(wantManifest, gotManifest); diff != "" {
						t.Errorf("Unexpected manifest (-want +got):\n%s", diff)
					}
				})

				t.Run("invalid manifest", func(t *testing.T) {
					t.Parallel()
					m, kvs := newKVStoreManifest(test.keyPrefix)
					kvs[path.Join(test.keyPrefix, "dsp-manifest.json")] = []byte("bogus non-json data")
					_, err := m.GetDataShareProcessorSpecificManifest(ctx, dspName)
					const wantErrStr = "couldn't unmarshal"
					if err == nil || !strings.Contains(err.Error(), wantErrStr) {
						t.Errorf("Wanted error containing %q, got: %v", wantErrStr, err)
					}
				})
			})

			t.Run("GetIngestorGlobalManifest", func(t *testing.T) {
				t.Parallel()
				t.Run("valid manifest", func(t *testing.T) {
					t.Parallel()
					m, kvs := newKVStoreManifest(test.keyPrefix)
					kvs[path.Join(test.keyPrefix, "global-manifest.json")] = globalManifestBytes
					gotManifest, err := m.GetIngestorGlobalManifest(ctx)
					if err != nil {
						t.Fatalf("Unexpected error from GetIngestorGlobalManifest: %v", err)
					}
					if diff := cmp.Diff(globalManifest, gotManifest); diff != "" {
						t.Errorf("Unexpected manifest (-want +got):\n%s", diff)
					}
				})

				t.Run("no manifest", func(t *testing.T) {
					t.Parallel()
					m, _ := newKVStoreManifest(test.keyPrefix)
					if _, err := m.GetIngestorGlobalManifest(ctx); !errors.Is(err, ErrObjectNotExist) {
						t.Errorf("Unexpected error from GetIngestorGlobalManifest: %v", err)
					}
				})

				t.Run("invalid manifest", func(t *testing.T) {
					t.Parallel()
					m, kvs := newKVStoreManifest(test.keyPrefix)
					kvs[path.Join(test.keyPrefix, "global-manifest.json")] = []byte("bogus non-json data")
					_, err := m.GetIngestorGlobalManifest(ctx)
					const wantErrStr = "couldn't unmarshal"
					if err == nil || !strings.Contains(err.Error(), wantErrStr) {
						t.Errorf("Unexpected error from GetIngestorGlobalManifest: %v", err)
					}
				})
			})
		})
	}
}

// newKVStoreManifest returns a new kvStoreManifest, backed by an in-memory map from keys to
// values that is also returned. Operations on the manifest will modify the
// map, and modifications to the map will be reflected by the manifest.
func newKVStoreManifest(keyPrefix string) (_ kvStoreManifest, kvs map[string][]byte) {
	kvs = map[string][]byte{}
	return kvStoreManifest{
		kv:        memKV{kvs},
		keyPrefix: keyPrefix,
	}, kvs
}

// memKV is an in-memory implementation of kvStore, suitable for testing.
type memKV struct{ kvs map[string][]byte }

var _ kvStore = memKV{} // verify memDS satisfies kvStore interface

func (kv memKV) put(_ context.Context, key string, data []byte) error {
	v := make([]byte, len(data))
	copy(v, data)
	kv.kvs[key] = v
	return nil
}

func (kv memKV) get(_ context.Context, key string) ([]byte, error) {
	v, ok := kv.kvs[key]
	if !ok {
		return nil, ErrObjectNotExist
	}
	data := make([]byte, len(v))
	copy(data, v)
	return data, nil
}
