package secrets

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/pem"
	"fmt"
	"io"
	"math/big"

	corev1 "k8s.io/api/core/v1"
)

type PrioKey struct {
	key            *ecdsa.PrivateKey
	KubeIdentifier *string
	Expiration     *string
}

type Unmarshaler = func([]byte) (*PrioKey, error)

func NewPrioKey() (*PrioKey, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("failed to generate P-256 key: %w", err)
	}

	return &PrioKey{
		key: key,
	}, nil
}

func NewKeyFromKubernetes(secret *corev1.Secret, unmarshaler Unmarshaler) (*PrioKey, error) {
	encodedKey := secret.Data[secretKeyMap]
	key := make([]byte, base64.StdEncoding.DecodedLen(len(encodedKey)))
	_, err := base64.StdEncoding.Decode(key, secret.Data[secretKeyMap])
	if err != nil {
		return nil, fmt.Errorf("unable to decode old secret key: %w", err)
	}

	prioKey, err := unmarshaler(key)
	if err != nil {
		return nil, fmt.Errorf("Unable to parse key %s: %w", secret.Name, err)
	}
	prioKey.KubeIdentifier = &secret.Name

	return prioKey, nil
}

func PrioKeyFromX962UncompressedKey(key []byte) (*PrioKey, error) {
	publicKey := key[:65]
	d := key[65:]

	x, y := elliptic.Unmarshal(elliptic.P256(), publicKey)

	dInt := new(big.Int)
	k := dInt.SetBytes(d)

	priv := new(ecdsa.PrivateKey)

	priv.PublicKey.Curve = elliptic.P256()
	priv.D = k
	priv.PublicKey.X = x
	priv.PublicKey.Y = y

	return &PrioKey{
		key: priv,
	}, nil
}

func PrioKeyFromPKCS8PrivateKey(key []byte) (*PrioKey, error) {
	k, err := x509.ParsePKCS8PrivateKey(key)

	return &PrioKey{
		key: k.(*ecdsa.PrivateKey),
	}, err
}

func (p *PrioKey) marshallX962UncompressedPrivateKey() []byte {
	marshalledPublicKey := elliptic.Marshal(elliptic.P256(), p.key.PublicKey.X, p.key.PublicKey.Y)
	return append(marshalledPublicKey, p.key.D.Bytes()...)
}

func (p *PrioKey) marshalPKCS8PrivateKey() ([]byte, error) {
	return x509.MarshalPKCS8PrivateKey(p.key)
}

func (p *PrioKey) GetPemEncodedPublicKey() (string, error) {
	pkixPublic, err := x509.MarshalPKIXPublicKey(p.key.Public())
	if err != nil {
		return "", fmt.Errorf("failed to marshall ECDSA public key to PKIX %w", err)
	}

	block := &pem.Block{
		Type:  "PUBLIC KEY",
		Bytes: pkixPublic,
	}
	return string(pem.EncodeToMemory(block)), nil
}

func (p *PrioKey) CreateCertificateRequest(rand io.Reader, template *x509.CertificateRequest) (csr []byte, err error) {
	return x509.CreateCertificateRequest(rand, template, p.key)
}

func (p *PrioKey) CreatePemEncodedCertificateRequest(rand io.Reader, template *x509.CertificateRequest) (string, error) {
	csr, err := p.CreateCertificateRequest(rand, template)
	if err != nil {
		return "", err
	}

	b := &pem.Block{
		Type:  "CERTIFICATE REQUEST",
		Bytes: csr,
	}

	return string(pem.EncodeToMemory(b)), nil
}
