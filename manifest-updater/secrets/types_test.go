package secrets

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/pem"
	"testing"
)

func TestPrioKeyMarshallAndUnmarshall(t *testing.T) {
	p256Key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)

	if err != nil {
		t.Errorf("error generating key: %s", err)
	}

	key := PrioKey{key: p256Key}
	x92 := key.marshallX962UncompressedPrivateKey()

	newKey := PrioKeyFromX962UncompressedKey(x92)

	equality := key.key.Equal(newKey.key)

	if !equality {
		t.Errorf("generated key and deseralized key not the same")
	}
}

func TestPrioKey(t *testing.T) {
	pemCsr := "-----BEGIN CERTIFICATE REQUEST-----\nMIHuMIGVAgEAMDMxMTAvBgNVBAMTKHVzLWN0LnByb2QtdXMuY2VydGlmaWNhdGVz\nLmlzcmctcHJpby5vcmcwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAATp7xFbJGwH\neMlAW8W0cQ57qCPIBT5NBr2jR8a+Z1/QzQJRtJvR2pqbaJhWKw7y9ogp/TmcsaX+\noP74+SSGwrEYoAAwCgYIKoZIzj0EAwIDSAAwRQIgLSekh4unn6fLv9O9K4Lr6VxG\nEpLSqFz259+Lrk7lwOkCIQCOzNvxwSb+iVFxJkaxUnxGYp2J+/2OnDGsKpyWY/wd\nhg==\n-----END CERTIFICATE REQUEST-----\n"
	pemBlock, rest := pem.Decode([]byte(pemCsr))
	if len(rest) != 0 {
		t.Errorf("Error state")
	}
	if pemBlock.Type != "CERTIFICATE REQUEST" {
		t.Errorf("Error state")
	}
	t.Log(pemBlock)
	csr, err := x509.ParseCertificateRequest(pemBlock.Bytes)
	if err != nil {
		t.Errorf("Error state")
	}
	t.Log(csr)
}

func TestCSRDecode(t *testing.T) {
	privateKey := "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw=="
	privateKeyBytes := make([]byte, base64.StdEncoding.DecodedLen(len(privateKey)))

	expectedPublicKey := "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVQ="

	_, _ = base64.StdEncoding.Decode(privateKeyBytes, []byte(privateKey))

	prioKey := PrioKeyFromX962UncompressedKey(privateKeyBytes)
	csr, _ := prioKey.CreateCertificateRequest(rand.Reader, new(x509.CertificateRequest))
	csrDecoded, _ := x509.ParseCertificateRequest(csr)

	ecdsaPublicKey := csrDecoded.PublicKey.(*ecdsa.PublicKey)
	newPublicKeyBytes := elliptic.Marshal(ecdsaPublicKey.Curve, ecdsaPublicKey.X, ecdsaPublicKey.Y)

	newPublicKey := base64.StdEncoding.EncodeToString(newPublicKeyBytes)
	validity := expectedPublicKey == newPublicKey
	if !validity {
		t.Errorf("Values weren't equal")
	}
}
