package key

import (
	"crypto/ecdsa"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"io"
)

type PrioKey struct {
	key *ecdsa.PrivateKey
}

func NewPrioKey(key *ecdsa.PrivateKey) *PrioKey {
	return &PrioKey{
		key: key,
	}
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

func GetPrioCSRTemplate(commonName string) *x509.CertificateRequest {
	csrTemplate := new(x509.CertificateRequest)

	csrTemplate.SignatureAlgorithm = x509.SHA256WithRSA
	csrTemplate.Subject = pkix.Name{
		CommonName: commonName,
	}

	return csrTemplate
}
