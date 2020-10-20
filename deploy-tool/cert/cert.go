package cert

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/x509"
	"deploy-tool/config"
	"deploy-tool/dns"
	"fmt"
	"github.com/caddyserver/certmagic"
	"strings"
)

// IssueCertificate asks the Let's Encrypt ACMEApiEndpoint to validate and sign a certificate for a site using the DNS01 strategy
func IssueCertificate(deployConfig config.DeployConfig, site string, privKey *ecdsa.PrivateKey) (string, error) {
	if deployConfig.ACME.SubscriberAgreement == false {
		return "", fmt.Errorf("let's encrypt ACMEApiEndpoint terms were not agreed to")
	}

	dnsProvider, err := dns.GetACMEDNSProvider(deployConfig)

	if err != nil {
		return "", fmt.Errorf("error when getting the dns provider: %v", err)
	}

	certmagic.DefaultACME.DNS01Solver = &certmagic.DNS01Solver{
		DNSProvider: dnsProvider,
	}

	certmagic.DefaultACME.Agreed = true
	certmagic.DefaultACME.Email = deployConfig.ACME.Email
	certmagic.DefaultACME.CA = deployConfig.ACME.ACMEApiEndpoint

	acme := certmagic.NewDefault()
	if err := setStorageDriver(deployConfig, acme); err != nil {
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

func setStorageDriver(deployConfig config.DeployConfig, acme *certmagic.Config) error {
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
