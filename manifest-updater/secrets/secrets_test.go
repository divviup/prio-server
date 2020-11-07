package secrets

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"testing"
)

func TestPrioKeyMarshallAndUnmarshall(t *testing.T) {
	p256Key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)

	if err != nil {
		t.Errorf("error generating key: %w", err)
	}

	key := PrioKey{key: p256Key}
	x92 := key.MarshallX962UncompressedPrivateKey()

	newKey := PrioKeyFromX962UncompressedKey(x92)

	key.key.Equal(newKey.key)
}
