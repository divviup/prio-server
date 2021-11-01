// Package key contains functionality for working with versioned Prio keys.
package key

import (
	"errors"
	"fmt"
	"sort"
	"strconv"
	"time"
)

// Key represents a cryptographic key. It may be "versioned": there may be
// multiple pieces of key material, any of which should be considered for use
// in decryption or signature verification. A single version will be considered
// "primary"; this version will be used for encryption or signing.
type Key map[string]Version

// Equal returns true if and only if this Key is equal to the given Key.
func (k Key) Equal(o Key) bool {
	if len(k) != len(o) {
		return false
	}
	for kid, kv := range k {
		ov, ok := o[kid]
		if !ok || !kv.Equal(ov) {
			return false
		}
	}
	return true
}

// RotationConfig defines the configuration for a key-rotation operation.
type RotationConfig struct {
	CreateKeyFunc func() (string, error) // CreateKeyFunc returns newly-generated (private) key material, or returns an error if it can't.
	CreateMinAge  time.Duration          // CreateMinAge is the minimum age of the youngest key version before a new key version will be created.

	PrimaryMinAge time.Duration // PrimaryMinAge is the minimum age of a key version before it may normally be considered "primary".

	DeleteMinAge      time.Duration // DeleteMinAge is the minimum age of a key version before it will be considered for deletion.
	DeleteMinKeyCount int           // DeleteMinKeyCount is the minimum number of key versions before any key versions will be considered for deletion.
}

// Validate validates the rotation config, returning an error if and only if
// there is some problem with the specified configuration parameters.
func (cfg RotationConfig) Validate() error {
	// Create parameters
	if cfg.CreateKeyFunc == nil {
		return errors.New("CreateKeyFunc must be set")
	}
	if cfg.CreateMinAge < 0 {
		return errors.New("CreateMinAge must be non-negative")
	}

	// Primary parameters
	if cfg.PrimaryMinAge < 0 {
		return errors.New("PrimaryMinAge must be non-negative")
	}

	// Delete parameters
	if cfg.DeleteMinAge < 0 {
		return errors.New("DeleteMinAge must be non-negative")
	}
	if cfg.DeleteMinKeyCount < 0 {
		return errors.New("DeleteMinKeys must be non-negative")
	}

	return nil
}

// Rotate potentially rotates the key according to the provided rotation
// config, returning a new key (or the same key, if no rotation is necessary).
//
// Keys are rotated according to the following policy:
//  * If no key versions exist, or if the youngest key version is older than
//    `create_min_age`, create a new key version.
//  * While there are more than `delete_min_key_count` keys, and the oldest key
//    version is older than `delete_min_age`, delete the oldest key version.
//  * Mark a single key version as primary (unmarking any other key versions
//    that may be marked primary):
//    * If there is a key version older than `primary_min_age`, mark the
//      youngest such key version as primary.
//    * Otherwise, mark the oldest key version as primary.
func (k Key) Rotate(now time.Time, cfg RotationConfig) (Key, error) {
	// Validate parameters.
	if err := cfg.Validate(); err != nil {
		return Key{}, fmt.Errorf("invalid rotation config: %w", err)
	}

	// Turn the map of key identifier -> version into a list of (identifier,
	// version) tuples, sorted by creation time. Also, validate that we aren't
	// trying to rotate a key containing a version from the "future" to
	// simplify later logic, and go ahead and unmark primary on all key
	// versions so that we can easily mark a single version primary later.
	type kidVersion struct {
		kid string
		ver Version
	}
	age := func(kv kidVersion) time.Duration { return now.Sub(kv.ver.CreationTime) }
	kvs := make([]kidVersion, 0, 1+len(k))
	for kid, ver := range k {
		kv := kidVersion{kid, ver}
		kv.ver.Primary = false
		if age(kv) < 0 {
			return Key{}, fmt.Errorf("key version %q has creation time %v, after now (%v)", kid, ver.CreationTime.Format(time.RFC3339), now.Format(time.RFC3339))
		}
		kvs = append(kvs, kv)
	}
	sort.Slice(kvs, func(i, j int) bool { return kvs[i].ver.CreationTime.Before(kvs[j].ver.CreationTime) })

	// Policy: if no key versions exist, or if the youngest key version is
	// older than `create_min_age`, create a new key version.
	if len(kvs) == 0 || age(kvs[len(kvs)-1]) > cfg.CreateMinAge {
		newKid := strconv.FormatInt(now.Unix(), 10)
		newKeyMaterial, err := cfg.CreateKeyFunc()
		if err != nil {
			return Key{}, fmt.Errorf("couldn't create new key material: %w", err)
		}
		kvs = append(kvs, kidVersion{newKid, Version{KeyMaterial: newKeyMaterial, CreationTime: now}})
	}

	// Policy: While there are more than `delete_min_key_count` keys, and the
	// oldest key version is older than `delete_min_age`, delete the oldest key
	// version.
	for len(kvs) > cfg.DeleteMinKeyCount && age(kvs[0]) > cfg.DeleteMinAge {
		kvs = kvs[1:]
	}

	// Policy: determine & mark the current primary key version (unmarking any
	// versions that were previously marked primary):
	//  * If there is a key version older than `primary_min_age`, select the
	//    youngest such key version.
	//  * Otherwise, select the oldest key version.
	// This is implemented as a binary search which returns the index of the
	// first key version that is younger than `primary_min_age`. If this index
	// is 0, all key versions are younger than `primary_min_age`, so we want to
	// use the oldest key version, i.e. the one in index 0. If this is not
	// zero, we want to use the next key version older than the one we found,
	// i.e. the one in the preceding index.
	primaryIdx := sort.Search(len(kvs), func(i int) bool { return age(kvs[i]) <= cfg.PrimaryMinAge })
	if primaryIdx > 0 {
		primaryIdx--
	}
	kvs[primaryIdx].ver.Primary = true

	// Transform the sorted list of (identifier, version) tuples back into a
	// Key, and return it.
	newKey := Key{}
	for _, kv := range kvs {
		newKey[kv.kid] = kv.ver
	}
	return newKey, nil
}

// Version represents a single version of a key, i.e. raw private key material,
// as well as associated metadata. Typically, a Version will be embedded within
// a Set.
type Version struct {
	KeyMaterial  string    `json:"key,omitempty"`
	CreationTime time.Time `json:"creation_time,omitempty"`
	Primary      bool      `json:"priamry,omitempty"`
}

// Equal returns true if and only if this Version is equal to the given
// Version.
func (v Version) Equal(o Version) bool {
	return v.KeyMaterial == o.KeyMaterial &&
		v.CreationTime.Equal(o.CreationTime) &&
		v.Primary == o.Primary
}
