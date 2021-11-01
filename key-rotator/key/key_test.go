package key

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"
)

func TestKeyMarshal(t *testing.T) {
	t.Parallel()

	mustKey := func(r Raw, err error) Raw {
		if err != nil {
			t.Fatalf("Couldn't create key: %v", err)
		}
		return r
	}
	testKey0 := mustKey(Test.New())
	testKey1 := mustKey(Test.New())
	p256Key0 := mustKey(P256.New())
	p256Key1 := mustKey(P256.New())

	wantKey := Key{
		Version{
			RawKey:       testKey0,
			CreationTime: time.Unix(100000, 0).UTC(),
		},
		Version{
			RawKey:       p256Key0,
			CreationTime: time.Unix(150000, 0).UTC(),
		},
		Version{
			RawKey:       testKey1,
			CreationTime: time.Unix(200000, 0).UTC(),
			Primary:      true,
		},
		Version{
			RawKey:       p256Key1,
			CreationTime: time.Unix(250000, 0).UTC(),
		},
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
}

func TestKeyRotate(t *testing.T) {
	t.Parallel()

	const now = 100000

	cfg := RotationConfig{
		CreateKeyFunc: func() (Raw, error) { return newTestKey(now), nil },
		CreateMinAge:  10000 * time.Second,

		PrimaryMinAge: 1000 * time.Second,

		DeleteMinAge:      20000 * time.Second,
		DeleteMinKeyCount: 2,
	}

	// Success tests.
	for _, test := range []struct {
		name    string
		key     Key
		wantKey Key
	}{
		// Basic creation tests.
		{
			name:    "no creation at boundary",
			key:     Key{pkv(90000)},
			wantKey: Key{pkv(90000)},
		},
		{
			name:    "creation",
			key:     Key{pkv(89999)},
			wantKey: Key{pkv(89999), kv(now)},
		},

		// Basic primary tests.
		{
			name:    "no new primary at boundary",
			key:     Key{pkv(90000), kv(99000)},
			wantKey: Key{pkv(90000), kv(99000)},
		},
		{
			name:    "new primary",
			key:     Key{pkv(90000), kv(98999)},
			wantKey: Key{kv(90000), pkv(98999)},
		},

		// Basic deletion tests.
		{
			name:    "no deletion at boundary",
			key:     Key{kv(80000), kv(97000), pkv(98000)},
			wantKey: Key{kv(80000), kv(97000), pkv(98000)},
		},
		{
			name:    "no deletion at min key count",
			key:     Key{kv(79999), pkv(98000)},
			wantKey: Key{kv(79999), pkv(98000)},
		},
		{
			name:    "deletion",
			key:     Key{kv(79999), kv(97000), pkv(98000)},
			wantKey: Key{kv(97000), pkv(98000)},
		},

		// Miscellaneous tests.
		{
			name:    "empty key",
			key:     Key{},
			wantKey: Key{pkv(now)},
		},
		{
			name:    "creation, new primary, and deletion",
			key:     Key{pkv(79999), kv(89999)},
			wantKey: Key{pkv(89999), kv(100000)},
		},
	} {
		test := test
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()

			// Check that we get the wanted key from Rotate.
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
		_, err := Key{pkv(100001)}.Rotate(time.Unix(now, 0), cfg)
		if err == nil || !strings.Contains(err.Error(), wantErrString) {
			t.Errorf("Wanted error containing %q, got: %v", wantErrString, err)
		}
	})
	t.Run("key creation function returns error", func(t *testing.T) {
		t.Parallel()
		const wantErrString = "bananas"
		cfg := cfg
		cfg.CreateKeyFunc = func() (Raw, error) { return Raw{}, errors.New(wantErrString) }
		_, err := Key{}.Rotate(time.Unix(now, 0), cfg)
		if err == nil || !strings.Contains(err.Error(), wantErrString) {
			t.Errorf("Wanted error containing %q, got: %v", wantErrString, err)
		}
	})
}

// kv creates a non-primary key version with the given timestamp and bogus key material.
func kv(ts int64) Version {
	return Version{
		RawKey:       newTestKey(ts),
		CreationTime: time.Unix(ts, 0),
	}
}

// pkv creates a primary key version with the given timestamp and bogus key material.
func pkv(ts int64) Version {
	kv := kv(ts)
	kv.Primary = true
	return kv
}
