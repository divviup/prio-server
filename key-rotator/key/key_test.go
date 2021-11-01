package key

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"
)

func TestKeyRotate(t *testing.T) {
	t.Parallel()

	const now = 100000

	cfg := RotationConfig{
		CreateKeyFunc: func() (string, error) { return kv(now).KeyMaterial, nil },
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
			key:     key(pkv(90000)),
			wantKey: key(pkv(90000)),
		},
		{
			name:    "creation",
			key:     key(pkv(89999)),
			wantKey: key(pkv(89999), kv(now)),
		},

		// Basic primary tests.
		{
			name:    "no new primary at boundary",
			key:     key(pkv(90000), kv(99000)),
			wantKey: key(pkv(90000), kv(99000)),
		},
		{
			name:    "new primary",
			key:     key(pkv(90000), kv(98999)),
			wantKey: key(kv(90000), pkv(98999)),
		},

		// Basic deletion tests.
		{
			name:    "no deletion at boundary",
			key:     key(kv(80000), kv(97000), pkv(98000)),
			wantKey: key(kv(80000), kv(97000), pkv(98000)),
		},
		{
			name:    "no deletion at min key count",
			key:     key(kv(79999), pkv(98000)),
			wantKey: key(kv(79999), pkv(98000)),
		},
		{
			name:    "deletion",
			key:     key(kv(79999), kv(97000), pkv(98000)),
			wantKey: key(kv(97000), pkv(98000)),
		},

		// Miscellaneous tests.
		{
			name:    "empty key",
			key:     key(),
			wantKey: key(pkv(now)),
		},
		{
			name:    "creation, new primary, and deletion", // oh my
			key:     key(pkv(79999), kv(89999)),
			wantKey: key(pkv(89999), kv(100000)),
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
		_, err := key(pkv(100001)).Rotate(time.Unix(now, 0), cfg)
		if err == nil || !strings.Contains(err.Error(), wantErrString) {
			t.Errorf("Wanted error containing %q, got: %v", wantErrString, err)
		}
	})
	t.Run("key creation function returns error", func(t *testing.T) {
		t.Parallel()
		const wantErrString = "bananas"
		cfg := cfg
		cfg.CreateKeyFunc = func() (string, error) { return "", errors.New(wantErrString) }
		_, err := key().Rotate(time.Unix(now, 0), cfg)
		if err == nil || !strings.Contains(err.Error(), wantErrString) {
			t.Errorf("Wanted error containing %q, got: %v", wantErrString, err)
		}
	})
}

// key creates a key with the given key versions, with key identifiers based on their timestamps.
func key(kvs ...Version) Key {
	k := Key{}
	for _, kv := range kvs {
		kid := strconv.FormatInt(kv.CreationTime.Unix(), 10)
		if _, ok := k[kid]; ok {
			panic(fmt.Sprintf("key provided duplicate creation time %q", kid))
		}
		k[kid] = kv
	}
	return k
}

// kv creates a non-primary key version with the given timestamp and bogus key material.
func kv(ts int64) Version {
	return Version{
		KeyMaterial:  fmt.Sprintf("key %d key material", ts),
		CreationTime: time.Unix(ts, 0),
	}
}

// pkv creates a primary key version with the given timestamp and bogus key material.
func pkv(ts int64) Version {
	kv := kv(ts)
	kv.Primary = true
	return kv
}
