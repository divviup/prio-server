package storage

import (
	"context"
	"encoding/json"
	"path"
	"strings"
	"testing"

	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	"github.com/google/go-cmp/cmp"
)

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
		t.Run("WriteDataShareProcessorSpecificManifest", func(t *testing.T) {
			t.Parallel()
			m, objs := newManifest(test.keyPrefix)
			wantObjs := map[string][]byte{path.Join(test.keyPrefix, "dsp-manifest.json"): dspManifestBytes}
			if err := m.WriteDataShareProcessorSpecificManifest(dspManifest, dspName); err != nil {
				t.Fatalf("Unexpected error from WriteDataShareProcessorSpecificManifest: %v", err)
			}
			if diff := cmp.Diff(wantObjs, objs); diff != "" {
				t.Errorf("Unexpected objects in datastore (-want +got):\n%s", diff)
			}
		})

		t.Run("WriteIngestorGlobalManifest", func(t *testing.T) {
			t.Parallel()
			m, objs := newManifest(test.keyPrefix)
			wantObjs := map[string][]byte{path.Join(test.keyPrefix, "global-manifest.json"): globalManifestBytes}
			if err := m.WriteIngestorGlobalManifest(globalManifest); err != nil {
				t.Fatalf("Unexpected error from WriteIngestorGlobalManifest: %v", err)
			}
			if diff := cmp.Diff(wantObjs, objs); diff != "" {
				t.Errorf("Unexpected objects in datastore (-want +got):\n%s", diff)
			}
		})

		t.Run("FetchDataShareProcessorSpecificManifest", func(t *testing.T) {
			t.Parallel()
			t.Run("valid manifest", func(t *testing.T) {
				t.Parallel()
				m, objs := newManifest(test.keyPrefix)
				objs[path.Join(test.keyPrefix, "dsp-manifest.json")] = dspManifestBytes
				gotManifest, err := m.FetchDataShareProcessorSpecificManifest(dspName)
				if err != nil {
					t.Fatalf("Unexpected error from FetchDataShareProcessorSpecificManifest: %v", err)
				}
				if diff := cmp.Diff(&dspManifest, gotManifest); diff != "" {
					t.Errorf("Unexpected manifest (-want +got):\n%s", diff)
				}
			})

			t.Run("invalid manifest", func(t *testing.T) {
				t.Parallel()
				m, objs := newManifest(test.keyPrefix)
				objs[path.Join(test.keyPrefix, "dsp-manifest.json")] = []byte("bogus non-json data")
				_, err := m.FetchDataShareProcessorSpecificManifest(dspName)
				const wantErrStr = "couldn't unmarshal"
				if err == nil || !strings.Contains(err.Error(), wantErrStr) {
					t.Errorf("Wanted error containing %q, got: %v", wantErrStr, err)
				}
			})
		})

		t.Run("IngestorGlobalManifestExists", func(t *testing.T) {
			t.Parallel()
			t.Run("valid manifest", func(t *testing.T) {
				t.Parallel()
				m, objs := newManifest(test.keyPrefix)
				objs[path.Join(test.keyPrefix, "global-manifest.json")] = globalManifestBytes
				const wantExists = true
				gotExists, err := m.IngestorGlobalManifestExists()
				if err != nil {
					t.Fatalf("Unexpected error from IngestorGlobalManifestExists: %v", err)
				}
				if wantExists != gotExists {
					t.Errorf("IngestorGlobalManifestExists() = %v, want %v", gotExists, wantExists)
				}
			})

			t.Run("no manifest", func(t *testing.T) {
				t.Parallel()
				m, _ := newManifest(test.keyPrefix)
				const wantExists = false
				gotExists, err := m.IngestorGlobalManifestExists()
				if err != nil {
					t.Fatalf("Unexpected error from IngestorGlobalManifestExists: %v", err)
				}
				if wantExists != gotExists {
					t.Errorf("IngestorGlobalManifestExists() = %v, want %v", gotExists, wantExists)
				}
			})

			t.Run("invalid manifest", func(t *testing.T) {
				t.Parallel()
				m, objs := newManifest(test.keyPrefix)
				objs[path.Join(test.keyPrefix, "global-manifest.json")] = []byte("bogus non-json data")
				_, err := m.IngestorGlobalManifestExists()
				const wantErrStr = "couldn't unmarshal"
				if err == nil || !strings.Contains(err.Error(), wantErrStr) {
					t.Errorf("Unexpected error from IngestorGlobalManifestExists: %v", err)
				}
			})
		})
	}
}

// newManifest returns a new Manifest, backed by a map that is also returned.
// Operations on the Manifest will modify the map, and modifications to the map
// will be reflected by the Manifest.
func newManifest(keyPrefix string) (_ Manifest, objs map[string][]byte) {
	objs = map[string][]byte{}
	ds := memDS{objs}
	return Manifest{ds, keyPrefix}, objs
}

// memDS is an in-memory implementation of datastore, suitable for testing.
type memDS struct{ objs map[string][]byte }

var _ datastore = memDS{} // verify memDS satisfies datastore interface

func (ds memDS) put(_ context.Context, key string, data []byte) error {
	d := make([]byte, len(data))
	copy(d, data)
	ds.objs[key] = d
	return nil
}

func (ds memDS) get(_ context.Context, key string) ([]byte, error) {
	d, ok := ds.objs[key]
	if !ok {
		return nil, errObjectNotExist
	}
	data := make([]byte, len(d))
	copy(data, d)
	return data, nil
}
