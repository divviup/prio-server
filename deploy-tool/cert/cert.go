package cert

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/x509"
	"fmt"
	"strings"

	"github.com/abetterinternet/prio-server/deploy-tool/config"
	"github.com/abetterinternet/prio-server/deploy-tool/dns"
	"github.com/caddyserver/certmagic"
)

// CertificateManager is a structure that defines the Certificate Issuance and renewal implementation
type CertificateManager struct {
	Config config.DeployConfig
}

func (certManager *CertificateManager) getCertMagic() (*certmagic.Config, error) {
	deployConfig := certManager.Config
	dnsProvider, err := dns.GetACMEDNSProvider(certManager.Config)

	if err != nil {
		return nil, fmt.Errorf("error when getting the dns provider: %v", err)
	}

	certmagic.DefaultACME.DNS01Solver = &certmagic.DNS01Solver{
		DNSProvider: dnsProvider,
	}

	certmagic.DefaultACME.Agreed = deployConfig.ACME.SubscriberAgreement
	certmagic.DefaultACME.Email = deployConfig.ACME.Email
	certmagic.DefaultACME.CA = deployConfig.ACME.ACMEApiEndpoint

	acme := certmagic.NewDefault()
	if err := certManager.setStorageDriver(acme); err != nil {
		return nil, err
	}

	return acme, nil
}

func (certManager *CertificateManager) setStorageDriver(acme *certmagic.Config) error {
	deployConfig := certManager.Config
	switch strings.ToLower(deployConfig.Storage.Driver) {
	case "filesystem":
		if deployConfig.Storage.Filesystem == nil {
			return fmt.Errorf("fileystem configuration was nil")
		}

		acme.Storage = &certmagic.FileStorage{Path: deployConfig.Storage.Filesystem.Path}
	case "kubernetes":
		if deployConfig.Storage.Kubernetes == nil {
			return fmt.Errorf("kubernetes configuration was nil")
		}

		secretStorage, err := NewKubernetesSecretStorage(deployConfig.Storage.Kubernetes.Namespace)
		if err != nil {
			return err
		}

		acme.Storage = secretStorage
	}
	return nil
}

// IssueCertificate asks the Let's Encrypt ACMEApiEndpoint to validate and sign a certificate for a site using the DNS01 strategy
func (certManager *CertificateManager) IssueCertificate(site string, privKey *ecdsa.PrivateKey) (string, error) {
	acme, err := certManager.getCertMagic()
	if err != nil {
		return "", err
	}

	csr, err := getCSR([]string{site}, privKey)
	if err != nil {
		return "", fmt.Errorf("error when getting the certificate signing request: %v", err)
	}

	issued, err := acme.Issuer.Issue(context.Background(), csr)
	if err != nil {
		return "", fmt.Errorf("error when issuing the certificate: %v", err)
	}

	return string(issued.Certificate), nil
}

// getCSR creates an x509 CertificateRequest which includes the site this certificate will be used for
func getCSR(names []string, privateKey crypto.PrivateKey) (*x509.CertificateRequest, error) {
	csrTemplate := new(x509.CertificateRequest)

	csrTemplate.DNSNames = append(csrTemplate.DNSNames, names...)

	csrDER, err := x509.CreateCertificateRequest(rand.Reader, csrTemplate, privateKey)
	if err != nil {
		return nil, err
	}

	return x509.ParseCertificateRequest(csrDER)
}
