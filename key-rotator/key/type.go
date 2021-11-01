package key

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/pem"
	"fmt"
)

// Type represents a type of cryptographic key, e.g. P256. It has functionality
// related to creating new raw keys of its type as well as serialization of raw
// keys to various formats.
type Type interface {
	// New creates a new raw key of this type, returning the private key
	// material serialized as a string.
	New() (key string, _ error)

	// PublicKeyAsCSR takes raw private key material, as would be returned by
	// New, and returns a PEM-encoding of the ASN.1 DER-encoding of a PKCS#10
	// (RFC 2986) CSR over the public key, using the provided FQDN as the
	// common name for the request.
	PublicKeyAsCSR(key, csrFQDN string) (string, error)

	// PublicKeyAsPKIX takes raw private key material, as would be returned by
	// New, and returns a PEM-encoding of the ASN.1 DER-encoding of the key in
	// PKIX (RFC 5280) format.
	PublicKeyAsPKIX(key string) (string, error)

	// PrivateKeyAsX962Uncompressed takes raw private key material, as would be
	// returned by New, and returns a base64 encoding of the X9.62 uncompressed
	// encoding of the public key concatenated with the secret "D" scalar.
	PrivateKeyAsX962Uncompressed(key string) (string, error)

	// PrivateKeyAsPKCS8 takes raw private key material, as would be returned
	// by New, and returns a base64 encoding of the ASN.1 DER-encoding of the
	// key in PKCS#8 (RFC 5208) format.
	PrivateKeyAsPKCS8(key string) (string, error)
}

// P256 returns a key Type that represents FIPS 186-3 P-256 keys.
func P256() Type { return p256{} }

type p256 struct{}

var _ Type = p256{} // verify p256 is a Type

func (p256) New() (string, error) {
	// New's raw key format is an an unpadded base64-encoding of the ASN.1
	// DER-encoding of the key as an RFC 5915 Elliptic Curve Private Key
	// Structure.
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return "", fmt.Errorf("couldn't generate new key: %w", err)
	}
	keyBytes, err := x509.MarshalECPrivateKey(key)
	if err != nil {
		return "", fmt.Errorf("couldn't marshal new key: %w", err)
	}
	return base64.RawStdEncoding.EncodeToString(keyBytes), nil
}

func (p256) PublicKeyAsCSR(key, csrFQDN string) (string, error) {
	pk, err := parseP256(key)
	if err != nil {
		return "", fmt.Errorf("bad key: %w", err)
	}
	tmpl := &x509.CertificateRequest{
		SignatureAlgorithm: x509.ECDSAWithSHA256,
		Subject:            pkix.Name{CommonName: csrFQDN},
	}
	csrBytes, err := x509.CreateCertificateRequest(rand.Reader, tmpl, pk)
	if err != nil {
		return "", fmt.Errorf("couldn't create certificate request: %w", err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: csrBytes})), nil
}

func (p256) PublicKeyAsPKIX(key string) (string, error) {
	pk, err := parseP256(key)
	if err != nil {
		return "", fmt.Errorf("bad key: %w", err)
	}
	pubkeyBytes, err := x509.MarshalPKIXPublicKey(pk.Public())
	if err != nil {
		return "", fmt.Errorf("couldn't encode as PKIX: %w", err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: pubkeyBytes})), nil
}

func (p256) PrivateKeyAsX962Uncompressed(key string) (string, error) {
	pk, err := parseP256(key)
	if err != nil {
		return "", fmt.Errorf("bad key: %w", err)
	}
	return base64.StdEncoding.EncodeToString(append(elliptic.Marshal(elliptic.P256(), pk.X, pk.Y), pk.D.Bytes()...)), nil
}

func (p256) PrivateKeyAsPKCS8(key string) (string, error) {
	pk, err := parseP256(key)
	if err != nil {
		return "", fmt.Errorf("bad key: %w", err)
	}
	keyBytes, err := x509.MarshalPKCS8PrivateKey(pk)
	if err != nil {
		return "", fmt.Errorf("couldn't encode as PKCS#8: %w", err)
	}
	return base64.StdEncoding.EncodeToString(keyBytes), nil
}

func parseP256(key string) (*ecdsa.PrivateKey, error) {
	keyBytes, err := base64.RawStdEncoding.DecodeString(key)
	if err != nil {
		return nil, fmt.Errorf("couldn't base64-decode: %w", err)
	}
	parsedKey, err := x509.ParseECPrivateKey(keyBytes)
	if err != nil {
		return nil, fmt.Errorf("couldn't parse EC key structure: %w", err)
	}
	if parsedKey.Curve != elliptic.P256() {
		return nil, fmt.Errorf("parsed key was not a P-256 key (was %q)", parsedKey.Params().Name)
	}
	return parsedKey, nil
}
