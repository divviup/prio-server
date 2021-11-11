// Package key contains functionality for working with versioned Prio keys.
package key

import (
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"time"
)

// Key represents a cryptographic key. It may be "versioned": there may be
// multiple pieces of key material, any of which should be considered for use
// in decryption or signature verification. A single version will be considered
// "primary"; this version will be used for encryption or signing.
type Key struct{ v []Version }

// Verify expected interfaces are implemented by Key.
var _ json.Marshaler = Version{}
var _ json.Unmarshaler = &Version{}

// FromVersions creates a new key comprised of the given key versions.
func FromVersions(versions ...Version) (Key, error) {
	vs := make([]Version, len(versions))
	copy(vs, versions)
	return Key{vs}, nil
}

// Equal returns true if and only if this Key is equal to the given Key.
func (k Key) Equal(o Key) bool {
	if len(k.v) != len(o.v) {
		return false
	}
	for i := range k.v {
		if !k.v[i].Equal(o.v[i]) {
			return false
		}
	}
	return true
}

// IsEmpty returns true if and only if this is the empty key, i.e. the key with
// no versions.
func (k Key) IsEmpty() bool { return len(k.v) == 0 }

// Versions visits the versions contained within this key in an unspecified
// order, calling the provided function on each version. If the provided
// function returns an error, Versions returns that error unchanged. Otherwise,
// Versions will never return an error.
func (k Key) Versions(f func(Version) error) error {
	for _, v := range k.v {
		if err := f(v); err != nil {
			return err
		}
	}
	return nil
}

// Primary returns the primary version of the key. It panics if the key is the
// empty key.
func (k Key) Primary() Version {
	for _, v := range k.v {
		if v.Primary {
			return v
		}
	}
	panic("no primary version")
}

// RotationConfig defines the configuration for a key-rotation operation.
type RotationConfig struct {
	CreateKeyFunc func() (Material, error) // CreateKeyFunc returns newly-generated key material, or an error if it can't.
	CreateMinAge  time.Duration            // CreateMinAge is the minimum age of the youngest key version before a new key version will be created.

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

	// Copy the existing list of key versions, sorting by creation time. Also,
	// validate that we aren't trying to rotate a key containing a version from
	// the "future" to simplify later logic, and go ahead and unmark primary on
	// all key versions so that we can easily mark a single version primary
	// later.
	age := func(v Version) time.Duration { return now.Sub(v.CreationTime) }
	kvs := make([]Version, 0, 1+len(k.v))
	for _, v := range k.v {
		if age(v) < 0 {
			return Key{}, fmt.Errorf("found key version with creation time %v, after now (%v)", v.CreationTime.Format(time.RFC3339), now.Format(time.RFC3339))
		}
		v.Primary = false
		kvs = append(kvs, v)
	}
	sort.Slice(kvs, func(i, j int) bool { return kvs[i].CreationTime.Before(kvs[j].CreationTime) })

	// Policy: if no key versions exist, or if the youngest key version is
	// older than `create_min_age`, create a new key version.
	if len(kvs) == 0 || age(kvs[len(kvs)-1]) > cfg.CreateMinAge {
		m, err := cfg.CreateKeyFunc()
		if err != nil {
			return Key{}, fmt.Errorf("couldn't create new key version: %w", err)
		}
		kvs = append(kvs, Version{KeyMaterial: m, CreationTime: now.UTC()})
	}

	// Policy: While there are more than `delete_min_key_count` keys, and the
	// oldest key version is older than `delete_min_age`, delete the oldest key
	// version.
	for len(kvs) > cfg.DeleteMinKeyCount && age(kvs[0]) > cfg.DeleteMinAge {
		kvs = kvs[1:]
	}

	// Policy: determine & mark the current primary key version (unmarking any
	// versions that were previously marked primary):
	//  * If there is a key version not younger than `primary_min_age`, select
	//    the youngest such key version.
	//  * Otherwise, select the oldest key version.
	// This is implemented as a binary search which returns the index of the
	// first key version that is younger than `primary_min_age`. If this index
	// is 0, all key versions are younger than `primary_min_age`, so we want to
	// use the oldest key version, i.e. the one in index 0. If this index is
	// not zero, we want to use the next key version older than the one we
	// found, i.e. the one in the preceding index.
	primaryIdx := sort.Search(len(kvs), func(i int) bool { return age(kvs[i]) < cfg.PrimaryMinAge })
	if primaryIdx > 0 {
		primaryIdx--
	}
	kvs[primaryIdx].Primary = true

	// Transform the sorted list of (identifier, version) tuples back into a
	// Key, and return it.
	return Key{kvs}, nil
}

func (k Key) MarshalJSON() ([]byte, error) { return json.Marshal(k.v) }

func (k *Key) UnmarshalJSON(data []byte) error {
	var vs []Version
	if err := json.Unmarshal(data, &vs); err != nil {
		return fmt.Errorf("couldn't unmarshal JSON: %w", err)
	}
	*k = Key{vs}
	return nil
}

// Version represents a single version of a key, i.e. raw private key material,
// as well as associated metadata. Typically, a Version will be embedded within
// a Set.
type Version struct {
	KeyMaterial  Material
	CreationTime time.Time
	Primary      bool
}

// Verify expected interfaces are implemented by Version.
var _ json.Marshaler = Version{}
var _ json.Unmarshaler = &Version{}

// Equal returns true if and only if this Version is equal to the given
// Version.
func (v Version) Equal(o Version) bool {
	return v.KeyMaterial.Equal(o.KeyMaterial) &&
		v.CreationTime.Equal(o.CreationTime) &&
		v.Primary == o.Primary
}

type jsonVersion struct {
	KeyMaterial  Material `json:"key"`
	CreationTime int64    `json:"creation_time,string"`
	Primary      bool     `json:"primary,omitempty"`
}

func (v Version) MarshalJSON() ([]byte, error) {
	return json.Marshal(jsonVersion{
		KeyMaterial:  v.KeyMaterial,
		CreationTime: v.CreationTime.Unix(),
		Primary:      v.Primary,
	})
}

func (v *Version) UnmarshalJSON(data []byte) error {
	var jv jsonVersion
	if err := json.Unmarshal(data, &jv); err != nil {
		return fmt.Errorf("couldn't unmarshal raw structure: %w", err)
	}
	*v = Version{
		KeyMaterial:  jv.KeyMaterial,
		CreationTime: time.Unix(jv.CreationTime, 0),
		Primary:      jv.Primary,
	}
	return nil
}
