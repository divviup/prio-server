package key

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"
)

func TestKeyMarshal(t *testing.T) {
	t.Parallel()

	t.Run("SerializeDeserialize", func(t *testing.T) {
		mustKey := func(r Material, err error) Material {
			if err != nil {
				t.Fatalf("Couldn't create key: %v", err)
			}
			return r
		}
		wantKey := k(
			Version{
				KeyMaterial:  mustKey(Test.New()),
				CreationTime: time.Unix(100000, 0).UTC(),
			},
			Version{
				KeyMaterial:  mustKey(P256.New()),
				CreationTime: time.Unix(150000, 0).UTC(),
			},
			Version{
				KeyMaterial:  mustKey(Test.New()),
				CreationTime: time.Unix(200000, 0).UTC(),
				Primary:      true,
			},
			Version{
				KeyMaterial:  mustKey(P256.New()),
				CreationTime: time.Unix(250000, 0).UTC(),
			},
		)

		buf, err := json.Marshal(wantKey)
		if err != nil {
			t.Fatalf("Couldn't JSON-marshal key: %v", err)
		}

		var gotKey Key
		if err := json.Unmarshal(buf, &gotKey); err != nil {
			t.Fatalf("Couldn't JSON-unmarshal key: %v", err)
		}

		diff := cmp.Diff(wantKey, gotKey)
		if !wantKey.Equal(gotKey) {
			t.Errorf("gotKey differs from wantKey (-want +got):\n%s", diff)
		} else if diff != "" {
			t.Errorf("gotKey is Equal to wantKey, but cmp.Diff disagrees (-want +got):\n%s", diff)
		}
	})

	t.Run("DeserializeSerialize", func(t *testing.T) {
		// wantKey generated from a run of the SerializeDeserialize test.
		const wantKey = `[{"key":"ACdcLaKY8VsN","creation_time":"100000"},{"key":"AQMp62hRUAKqVHXfhwApjJPMV21kxQpb0OYqk7/IxU5etbiIdgHv1+d5EHApWJrCD0a/QI4RtPx0iOkjr1Pitwsp","creation_time":"150000"},{"key":"ACrYJ2YS9Oem","creation_time":"200000","primary":true},{"key":"AQKtg3k806wsd0ld/FUSjr+9B9ZjvNIjL4Thwp/olCLNTDIpxAWKwzYAuqyCcChbQ72AShRIQQOJgkSVT6kw/N9b","creation_time":"250000"}]`

		var k Key
		if err := json.Unmarshal([]byte(wantKey), &k); err != nil {
			t.Fatalf("Couldn't JSON-unmarshal key: %v", err)
		}
		gotKey, err := json.Marshal(k)
		if err != nil {
			t.Fatalf("Couldn't JSON-marshal key: %v", err)
		}

		if diff := cmp.Diff([]byte(wantKey), gotKey); diff != "" {
			t.Errorf("gotKey differs from wantKey (-want +got):\n%s", err)
		}
	})
}

func TestKeyRotate(t *testing.T) {
	t.Parallel()

	const now = 100000

	baseCFG := RotationConfig{
		CreateMinAge: 10000 * time.Second,

		PrimaryMinAge: 1000 * time.Second,

		DeleteMinAge:      20000 * time.Second,
		DeleteMinKeyCount: 2,
	}

	// Success tests.
	for _, test := range []struct {
		name    string
		key     Key
		wantKey Key
		cfg     RotationConfig // falls back to baseCFG if unspecified
	}{
		// Basic creation tests.
		{
			name:    "no creation at boundary",
			key:     k(pkv(90000)),
			wantKey: k(pkv(90000)),
		},
		{
			name:    "creation",
			key:     k(pkv(89999)),
			wantKey: k(pkv(89999), kv(now)),
		},

		// Basic primary tests.
		{
			name:    "no new primary at boundary",
			key:     k(pkv(90000), kv(99001)),
			wantKey: k(pkv(90000), kv(99001)),
		},
		{
			name:    "new primary",
			key:     k(pkv(90000), kv(99000)),
			wantKey: k(kv(90000), pkv(99000)),
		},

		// Basic deletion tests.
		{
			name:    "no deletion at boundary",
			key:     k(kv(80000), kv(97000), pkv(98000)),
			wantKey: k(kv(80000), kv(97000), pkv(98000)),
		},
		{
			name:    "no deletion at min key count",
			key:     k(kv(79999), pkv(98000)),
			wantKey: k(kv(79999), pkv(98000)),
		},
		{
			name:    "deletion",
			key:     k(kv(79999), kv(97000), pkv(98000)),
			wantKey: k(kv(97000), pkv(98000)),
		},

		// Miscellaneous tests.
		{
			name:    "empty key",
			key:     Key{},
			wantKey: k(pkv(now)),
		},
		{
			name:    "creation, new primary, and deletion",
			key:     k(pkv(79999), kv(89999)),
			wantKey: k(pkv(89999), kv(100000)),
		},
		{
			name:    "zero minimum primary age allows key to become primary immediately",
			key:     k(pkv(0)),
			wantKey: k(kv(0), pkv(100000)),
			cfg: RotationConfig{
				CreateMinAge: 10000 * time.Second,

				PrimaryMinAge: 0,

				DeleteMinAge:      20000 * time.Second,
				DeleteMinKeyCount: 2,
			},
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()

			// Check that we get the desired key from Rotate.
			cfg := test.cfg
			if cfg.CreateMinAge == 0 && cfg.PrimaryMinAge == 0 && cfg.DeleteMinAge == 0 && cfg.DeleteMinKeyCount == 0 {
				cfg = baseCFG
			}
			cfg.CreateKeyFunc = func() (Material, error) { return newTestKey(now), nil }
			gotKey, err := test.key.Rotate(time.Unix(now, 0), cfg)
			if err != nil {
				t.Fatalf("Unexpected error from Rotate: %v", err)
			}
			diff := cmp.Diff(test.wantKey, gotKey)
			if !gotKey.Equal(test.wantKey) {
				t.Errorf("gotKey differs from wantKey (-want +got):\n%s", diff)
			} else if diff != "" {
				t.Errorf("gotKey is Equal to wantKey, but cmp.Diff disagrees (-want +got):\n%s", diff)
			}

			// Check that Rotate is idempotent when called multiple times with the same timestamp, config.
			secondGotKey, err := gotKey.Rotate(time.Unix(now, 0), cfg)
			if err != nil {
				t.Fatalf("Unexpected error from second call to Rotate: %v", err)
			}
			diff = cmp.Diff(gotKey, secondGotKey)
			if !secondGotKey.Equal(gotKey) {
				t.Errorf("secondGotKey differs from gotKey (-got +secondGot):\n%s", diff)
			} else if diff != "" {
				t.Errorf("secondGotKey is Equal to gotKey, but cmp.Diff disagrees (-want +got):\n%s", diff)
			}
		})
	}

	// Failure tests.
	t.Run("key from the future", func(t *testing.T) {
		t.Parallel()
		const wantErrString = "after now"
		cfg := baseCFG
		cfg.CreateKeyFunc = func() (Material, error) { return newTestKey(now), nil }
		_, err := k(pkv(100001)).Rotate(time.Unix(now, 0), cfg)
		if err == nil || !strings.Contains(err.Error(), wantErrString) {
			t.Errorf("Wanted error containing %q, got: %v", wantErrString, err)
		}
	})
	t.Run("key creation function returns error", func(t *testing.T) {
		t.Parallel()
		const wantErrString = "bananas"
		cfg := baseCFG
		cfg.CreateKeyFunc = func() (Material, error) { return Material{}, errors.New(wantErrString) }
		_, err := Key{}.Rotate(time.Unix(now, 0), cfg)
		if err == nil || !strings.Contains(err.Error(), wantErrString) {
			t.Errorf("Wanted error containing %q, got: %v", wantErrString, err)
		}
	})
}

// k creates a new key or dies trying.
func k(vs ...Version) Key {
	k, err := FromVersions(vs...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key from versions: %v", err))
	}
	return k
}

// kv creates a non-primary key version with the given timestamp and bogus key material.
func kv(ts int64) Version {
	return Version{
		KeyMaterial:  newTestKey(ts),
		CreationTime: time.Unix(ts, 0),
	}
}

// pkv creates a primary key version with the given timestamp and bogus key material.
func pkv(ts int64) Version {
	kv := kv(ts)
	kv.Primary = true
	return kv
}
