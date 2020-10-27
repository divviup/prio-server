package key_generator

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"fmt"
)

func GenerateKey() (string, error) {
	ecdsaKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return "", fmt.Errorf("failed to generate ECDSA P256 key: %w", err)
	}

	pkcs8PrivateKey, err := x509.MarshalPKCS8PrivateKey(ecdsaKey)
	if err != nil {
		return "", fmt.Errorf("failed to marshal ECDSA key to PKCS#8 document: %w", err)
	}

	return base64.StdEncoding.EncodeToString(pkcs8PrivateKey), nil
}
