// Package key contains functionality for working with versioned Prio keys.
package key

import (
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
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

// Diff returns a human-readable string describing the differences from the
// given `o` key to this key, suitable for logging. Diff returns the empty
// string if and only if the two keys are equal.
func (k Key) Diff(o Key) string {
	// Build up structures allowing easy generation of diffs.
	var newPKTS, oldPKTS *int64
	infos := map[int64]struct{ oldMat, newMat *Material }{}
	for i, v := range k.v {
		v := v
		if i == 0 {
			newPKTS = &v.CreationTimestamp
		}
		info := infos[v.CreationTimestamp]
		info.newMat = &v.KeyMaterial
		infos[v.CreationTimestamp] = info
	}
	for i, v := range o.v {
		v := v
		if i == 0 {
			oldPKTS = &v.CreationTimestamp
		}
		info := infos[v.CreationTimestamp]
		info.oldMat = &v.KeyMaterial
		infos[v.CreationTimestamp] = info
	}
	tss := make([]int64, 0, len(infos))
	for ts := range infos {
		tss = append(tss, ts)
	}
	sort.Slice(tss, func(i, j int) bool { return tss[i] < tss[j] })

	// Generate primary-version diffs.
	var diffs []string
	switch {
	case newPKTS == nil && oldPKTS == nil:
		// no diff if both keys are empty
	case oldPKTS == nil:
		diffs = append(diffs, fmt.Sprintf("changed primary version none → %d", *newPKTS))
	case newPKTS == nil:
		diffs = append(diffs, fmt.Sprintf("changed primary version %d → none", *oldPKTS))
	case *oldPKTS != *newPKTS:
		diffs = append(diffs, fmt.Sprintf("changed primary version %d → %d", *oldPKTS, *newPKTS))
	}

	// Generate key version diffs.
	for _, ts := range tss {
		info := infos[ts]
		switch {
		case info.oldMat == nil:
			diffs = append(diffs, fmt.Sprintf("added version %d", ts))
		case info.newMat == nil:
			diffs = append(diffs, fmt.Sprintf("removed version %d", ts))
		case !info.oldMat.Equal(*info.newMat):
			diffs = append(diffs, fmt.Sprintf("modified key material for version %d", ts))
		}
	}
	return strings.Join(diffs, "; ")
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

	// Other conditions.
	if !(cfg.PrimaryMinAge <= cfg.CreateMinAge && cfg.CreateMinAge <= cfg.DeleteMinAge) {
		return errors.New("config must satisfy PrimaryMinAge <= CreateMinAge <= DeleteMinAge")
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
//
// The returned key is guaranteed to include at least one version.
func (k Key) Rotate(now time.Time, cfg RotationConfig) (Key, error) {
	// Validate parameters.
	if err := cfg.Validate(); err != nil {
		return Key{}, fmt.Errorf("invalid rotation config: %w", err)
	}

	// Copy the existing list of key versions, sorting by creation time
	// ascending (oldest to youngest). Also, validate that we aren't trying to
	// rotate a key containing a version from the "future" to simplify later
	// logic.
	nowTS := now.Unix()
	age := func(v Version) time.Duration { return time.Second * time.Duration(nowTS-v.CreationTimestamp) }
	vs := make([]Version, 0, 1+len(k.v))
	for _, v := range k.v {
		if age(v) < 0 {
			return Key{}, fmt.Errorf("found key version with creation timestamp %d, after now (%d)", v.CreationTimestamp, nowTS)
		}
		vs = append(vs, v)
	}
	sort.Slice(vs, func(i, j int) bool { return vs[i].CreationTimestamp < vs[j].CreationTimestamp })

	// Policy: if no key versions exist, or if the youngest key version is
	// older than `create_min_age`, create a new key version.
	// (The version at the largest index is guaranteed to be the youngest due
	// to the sort criteria.)
	youngestVersionIdx := len(vs) - 1
	if len(vs) == 0 || age(vs[youngestVersionIdx]) > cfg.CreateMinAge {
		m, err := cfg.CreateKeyFunc()
		if err != nil {
			return Key{}, fmt.Errorf("couldn't create new key version: %w", err)
		}
		vs = append(vs, Version{KeyMaterial: m, CreationTimestamp: nowTS})
	}

	// Policy: While there are more than `delete_min_key_count` keys, and the
	// oldest key version is older than `delete_min_age`, delete the oldest key
	// version.
	// (The version at index 0 is guaranteed to be the oldest version due to
	// the sort criteria.)
	for len(vs) > cfg.DeleteMinKeyCount && age(vs[0]) > cfg.DeleteMinAge {
		vs = vs[1:]
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
	if len(vs) > 0 {
		primaryIdx := sort.Search(len(vs), func(i int) bool { return age(vs[i]) < cfg.PrimaryMinAge })
		if primaryIdx > 0 {
			primaryIdx--
		}
		vs[0], vs[primaryIdx] = vs[primaryIdx], vs[0]
	}

	// Validate invariants & return key.
	if len(vs) == 0 {
		return Key{}, fmt.Errorf("key validation error: after rotation, key must contain at least one version")
	}
	newK, err := fromVersionSlice(vs)
	if err != nil {
		return Key{}, fmt.Errorf("key validation error: %w", err)
	}
	return newK, nil
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
				return errors.New("key validation error: serialized key contains multiple primary versions")
			}
			foundPrimary = true
		}
	}
	if !foundPrimary {
		return errors.New("key validation error: serialized key contains no primary versions")
	}
	var err error
	*k, err = fromVersionSlice(vs)
	if err != nil {
		return fmt.Errorf("key validation error: %w", err)
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
