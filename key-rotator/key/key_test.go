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
		t.Parallel()
		mustKey := func(r Material, err error) Material {
			if err != nil {
				t.Fatalf("Couldn't create key: %v", err)
			}
			return r
		}
		wantKey, err := FromVersions(
			Version{
				KeyMaterial:       mustKey(Test.New()),
				CreationTimestamp: 200000,
			},
			Version{
				KeyMaterial:       mustKey(Test.New()),
				CreationTimestamp: 100000,
			},
			Version{
				KeyMaterial:       mustKey(P256.New()),
				CreationTimestamp: 150000,
			},
			Version{
				KeyMaterial:       mustKey(P256.New()),
				CreationTimestamp: 250000,
			},
		)
		if err != nil {
			t.Fatalf("Couldn't create wantKey: %v", err)
		}

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
		t.Parallel()

		// wantKey generated from a run of the SerializeDeserialize test.
		const wantKey = `[{"key":"ACrYJ2YS9Oem","creation_time":"200000","primary":true},{"key":"AQKtg3k806wsd0ld/FUSjr+9B9ZjvNIjL4Thwp/olCLNTDIpxAWKwzYAuqyCcChbQ72AShRIQQOJgkSVT6kw/N9b","creation_time":"250000"},{"key":"AQMp62hRUAKqVHXfhwApjJPMV21kxQpb0OYqk7/IxU5etbiIdgHv1+d5EHApWJrCD0a/QI4RtPx0iOkjr1Pitwsp","creation_time":"150000"},{"key":"ACdcLaKY8VsN","creation_time":"100000"}]`

		var k Key
		if err := json.Unmarshal([]byte(wantKey), &k); err != nil {
			t.Fatalf("Couldn't JSON-unmarshal key: %v", err)
		}
		gotKey, err := json.Marshal(k)
		if err != nil {
			t.Fatalf("Couldn't JSON-marshal key: %v", err)
		}

		if diff := cmp.Diff([]byte(wantKey), gotKey); diff != "" {
			t.Errorf("gotKey differs from wantKey (-want +got):\n%s", diff)
		}
	})

	t.Run("DeserializeValidation", func(t *testing.T) {
		t.Parallel()

		for _, test := range []struct {
			name          string
			serializedKey string
			wantErrStr    string
		}{
			{
				name:          "no primary versions",
				serializedKey: `[{"key":"ACrYJ2YS9Oem","creation_time":"200000"},{"key":"ACdcLaKY8VsN","creation_time":"100000"}]`,
				wantErrStr:    "no primary versions",
			},
			{
				name:          "multiple primary versions",
				serializedKey: `[{"key":"ACrYJ2YS9Oem","creation_time":"200000","primary":true},{"key":"ACdcLaKY8VsN","creation_time":"100000","primary":true}]`,
				wantErrStr:    "multiple primary versions",
			},
			{
				name:          "multiple versions with same timestamp (primary)",
				serializedKey: `[{"key":"ACrYJ2YS9Oem","creation_time":"200000","primary":true},{"key":"ACdcLaKY8VsN","creation_time":"100000"},{"key":"ACrYJ2YS9Oem","creation_time":"200000"}]`,
				wantErrStr:    "multiple versions with creation timestamp",
			},
			{
				name:          "multiple versions with same timestamp (non-primary)",
				serializedKey: `[{"key":"ACrYJ2YS9Oem","creation_time":"200000","primary":true},{"key":"ACdcLaKY8VsN","creation_time":"100000"},{"key":"ACdcLaKY8VsN","creation_time":"100000"}]`,
				wantErrStr:    "multiple versions with creation timestamp",
			},
		} {
			test := test
			t.Run(test.name, func(t *testing.T) {
				t.Parallel()
				var k Key
				err := json.Unmarshal([]byte(test.serializedKey), &k)
				if err == nil || !strings.Contains(err.Error(), test.wantErrStr) {
					t.Errorf("Wanted error containing %q, got: %v", test.wantErrStr, err)
				}
			})
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
			key:     k(90000),
			wantKey: k(90000),
		},
		{
			name:    "creation",
			key:     k(89999),
			wantKey: k(89999, now),
		},

		// Basic primary tests.
		{
			name:    "no new primary at boundary",
			key:     k(90000, 99001),
			wantKey: k(90000, 99001),
		},
		{
			name:    "new primary",
			key:     k(90000, 99000),
			wantKey: k(99000, 90000),
		},

		// Basic deletion tests.
		{
			name:    "no deletion at boundary",
			key:     k(98000, 80000, 97000),
			wantKey: k(98000, 80000, 97000),
		},
		{
			name:    "no deletion at min key count",
			key:     k(98000, 79999),
			wantKey: k(98000, 79999),
		},
		{
			name:    "deletion",
			key:     k(98000, 79999, 97000),
			wantKey: k(98000, 97000),
		},

		// Miscellaneous tests.
		{
			name:    "empty key",
			key:     Key{},
			wantKey: k(now),
		},
		{
			name:    "creation, new primary, and deletion",
			key:     k(79999, 89999),
			wantKey: k(89999, 100000),
		},
		{
			name:    "zero minimum primary age allows key to become primary immediately",
			key:     k(50000),
			wantKey: k(100000, 50000),
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
		_, err := k(100001).Rotate(time.Unix(now, 0), cfg)
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

// k creates a new key or dies trying with the given version timestamps and
// bogus key material. pkvTS is the primary key version timestamp, vtss are the
// non-primary version timestamps.
func k(pkvTS int64, vtss ...int64) Key {
	pkv := Version{KeyMaterial: newTestKey(pkvTS), CreationTimestamp: pkvTS}
	vs := []Version{}
	for _, vts := range vtss {
		vs = append(vs, Version{KeyMaterial: newTestKey(vts), CreationTimestamp: vts})
	}
	k, err := FromVersions(pkv, vs...)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create key from versions: %v", err))
	}
	return k
}
