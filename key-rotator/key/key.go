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
// "primary": this version will be used for encryption or signing.
type Key struct {
	// structure of v: if v is not empty, the first element is the primary version.
	// note well: all new, non-empty Key values should be created via `fromVersionSlice`.
	v []Version
}

// Verify expected interfaces are implemented by Key.
var _ json.Marshaler = Key{}
var _ json.Unmarshaler = &Key{}

// FromVersions creates a new key comprised of the given key versions.
func FromVersions(primaryVersion Version, otherVersions ...Version) (Key, error) {
	vs := make([]Version, 1+len(otherVersions))
	vs[0] = primaryVersion
	copy(vs[1:], otherVersions)
	return fromVersionSlice(vs)
}

// fromVersionSlice produces a Key from a slice of versions, which (if
// non-empty) must include its primary version as the first version. This
// function "takes ownership" of `vs` in the sense that it reorders its
// contents, so callers may need to make a copy.
//
// Internally, all new (non-empty) Key values should be created via this
// method.
func fromVersionSlice(vs []Version) (Key, error) {
	if len(vs) == 0 {
		return Key{}, nil
	}

	// Re-sort the non-primary keys by creation time descending (youngest to
	// oldest), to get a canonical ordering that is also a fairly reasonable
	// default ordering for decryption/signature-verification attempts.
	nonPrimaryVs := vs[1:]
	sort.Slice(nonPrimaryVs, func(i, j int) bool { return nonPrimaryVs[j].CreationTimestamp < nonPrimaryVs[i].CreationTimestamp })

	// Validate that all key versions have distinct creation timestamps.
	pkTS := vs[0].CreationTimestamp
	for i, v := range nonPrimaryVs {
		if ts := v.CreationTimestamp; ts == pkTS || (i > 0 && ts == nonPrimaryVs[i-1].CreationTimestamp) {
			return Key{}, fmt.Errorf("key contains multiple versions with creation timestamp %d", ts)
		}
	}

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
// function returns an error, Versions stops visiting versions and returns that
// error. Otherwise, Versions will never return an error.
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
func (k Key) Primary() Version { return k.v[0] }

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
//  * Determine the current primary version:
//    * If there is a key version not younger than `primary_min_age`, select
//      the youngest such key version as primary.
//    * Otherwise, select the oldest key version as primary.
func (k Key) Rotate(now time.Time, cfg RotationConfig) (Key, error) {
	// Validate parameters.
	if err := cfg.Validate(); err != nil {
		return Key{}, fmt.Errorf("invalid rotation config: %w", err)
	}

	// Copy the existing list of key versions, sorting by creation time (oldest
	// to youngest). Also, validate that we aren't trying to rotate a key
	// containing a version from the "future" to simplify later logic.
	nowTS := now.Unix()
	age := func(v Version) time.Duration { return time.Second * time.Duration(nowTS-v.CreationTimestamp) }
	kvs := make([]Version, 0, 1+len(k.v))
	for _, v := range k.v {
		if age(v) < 0 {
			return Key{}, fmt.Errorf("found key version with creation timestamp %d, after now (%d)", v.CreationTimestamp, nowTS)
		}
		kvs = append(kvs, v)
	}
	sort.Slice(kvs, func(i, j int) bool { return kvs[i].CreationTimestamp < kvs[j].CreationTimestamp })

	// Policy: if no key versions exist, or if the youngest key version is
	// older than `create_min_age`, create a new key version.
	if len(kvs) == 0 || age(kvs[len(kvs)-1]) > cfg.CreateMinAge {
		m, err := cfg.CreateKeyFunc()
		if err != nil {
			return Key{}, fmt.Errorf("couldn't create new key version: %w", err)
		}
		kvs = append(kvs, Version{KeyMaterial: m, CreationTimestamp: nowTS})
	}

	// Policy: While there are more than `delete_min_key_count` keys, and the
	// oldest key version is older than `delete_min_age`, delete the oldest key
	// version.
	for len(kvs) > cfg.DeleteMinKeyCount && age(kvs[0]) > cfg.DeleteMinAge {
		kvs = kvs[1:]
	}

	// Policy: determine the current primary version:
	//  * If there is a key version not younger than `primary_min_age`, select
	//    the youngest such key version.
	//  * Otherwise, select the oldest key version.
	// This is implemented as a binary search which returns the index of the
	// first key version that is younger than `primary_min_age`. If this index
	// is 0, all key versions are younger than `primary_min_age`, so we want to
	// use the oldest key version, i.e. the one in index 0. If this index is
	// not zero, we want to use the next key version older than the one we
	// found, i.e. the one in the preceding index. The determined primary key
	// version is "selected" by swapping it into the 0'th index.
	primaryIdx := sort.Search(len(kvs), func(i int) bool { return age(kvs[i]) < cfg.PrimaryMinAge })
	if primaryIdx > 0 {
		primaryIdx--
	}
	kvs[0], kvs[primaryIdx] = kvs[primaryIdx], kvs[0]
	return fromVersionSlice(kvs)
}

func (k Key) MarshalJSON() ([]byte, error) {
	jvs := make([]jsonVersion, len(k.v))
	for i, v := range k.v {
		jvs[i] = jsonVersion{
			KeyMaterial:       v.KeyMaterial,
			CreationTimestamp: v.CreationTimestamp,
			Primary:           i == 0,
		}
	}
	return json.Marshal(jvs)
}

func (k *Key) UnmarshalJSON(data []byte) error {
	var jvs []jsonVersion
	if err := json.Unmarshal(data, &jvs); err != nil {
		return err
	}

	foundPrimary := false
	vs := make([]Version, len(jvs))
	for i, jv := range jvs {
		vs[i] = Version{
			KeyMaterial:       jv.KeyMaterial,
			CreationTimestamp: jv.CreationTimestamp,
		}
		if jv.Primary {
			vs[0], vs[i] = vs[i], vs[0]
			if foundPrimary {
				return errors.New("validation error: serialized key contains multiple primary versions")
			}
			foundPrimary = true
		}
	}
	if !foundPrimary {
		return errors.New("validation error: serialized key contains no primary versions")
	}
	var err error
	*k, err = fromVersionSlice(vs)
	if err != nil {
		return fmt.Errorf("validation error: %w", err)
	}
	return nil
}

// Version represents a single version of a key, i.e. raw private key material,
// as well as associated metadata. Typically, a Version will be embedded within
// a Key.
type Version struct {
	KeyMaterial       Material
	CreationTimestamp int64 // Unix seconds timestamp
}

// Equal returns true if and only if this Version is equal to the given
// Version.
func (v Version) Equal(o Version) bool {
	return v.KeyMaterial.Equal(o.KeyMaterial) &&
		v.CreationTimestamp == o.CreationTimestamp
}

// jsonVersion represents a single version of a key, as would be marshalled to
// JSON.
type jsonVersion struct {
	KeyMaterial       Material `json:"key"`
	CreationTimestamp int64    `json:"creation_time,string"`
	Primary           bool     `json:"primary,omitempty"`
}
