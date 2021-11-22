// Package test provides test utilities for working with keys.
package test

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"fmt"
	"hash/fnv"
	"math/rand"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

// Material generates deterministic key material based on the given `kid`. It
// is very likely that different `kid` values will produce different key
// material. Not secure, for testing use only.
func Material(kid string) key.Material {
	// Stretch `kid` into a deterministic, arbitrary stream of bytes.
	h := fnv.New64()
	h.Write([]byte(kid))
	rnd := rand.New(rand.NewSource(int64(h.Sum64()))) // nolint:gosec // Use of non-cryptographic RNG is purposeful here.

	// Use byte stream to generate a P256 key, and wrap it into a key.Material.
	privKey, err := ecdsa.GenerateKey(elliptic.P256(), rnd)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create new P256 key: %v", err))
	}
	m, err := key.P256MaterialFrom(privKey)
	if err != nil {
		panic(fmt.Sprintf("Couldn't create P256 material: %v", err))
	}
	return m
}
