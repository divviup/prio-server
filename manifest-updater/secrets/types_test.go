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
	x92 := key.marshallX962UncompressedPrivateKey()

	newKey, _ := PrioKeyFromX962UncompressedKey(x92)

	equality := key.key.Equal(newKey.key)

	if !equality {
		t.Errorf("generated key and deseralized key not the same")
	}
}
