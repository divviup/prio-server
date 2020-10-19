package cert

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/x509"
	"deploy-tool/dns"
	"fmt"
	"github.com/caddyserver/certmagic"
	"os"
	"strings"
)

// IssueCertificate asks the Let's Encrypt CA to validate and sign a certificate for a site using the DNS01 strategy
// This issue depends on multiple environment variables being set before being invoked:
// D_CA_AGREE, expecting a value of "true" - checks to see if the user has agreed to the CA terms
// D_CA_EMAIL, expecting an email address - this is used for the ACME account
// D_DNS_PROVIDER, expecting a DNS provider, e.g. "cloudflare" - this is used for the DNS challenge
// D_API_TOKEN, expecting an API token for the DNS provider - this is used to authenticate to the DNS provider
func IssueCertificate(site string, privKey *ecdsa.PrivateKey) (string, error) {
	if getUserCATermsAgreement() == false {
		return "", fmt.Errorf("let's encrypt CA terms were not agreed to")
	}

	userEmail, err := getUserEmail()
	if err != nil {
		return "", fmt.Errorf("error when getting the user email: %v", err)
	}

	dnsProvider, err := dns.GetACMEDNSProvider()

	if err != nil {
		return "", fmt.Errorf("error when getting the dns provider: %v", err)
	}

	certmagic.DefaultACME.DNS01Solver = &certmagic.DNS01Solver{
		DNSProvider: dnsProvider,
	}

	certmagic.DefaultACME.Agreed = true
	certmagic.DefaultACME.Email = userEmail
	// TODO change this to default LE server
	certmagic.DefaultACME.CA = certmagic.LetsEncryptStagingCA

	acme := certmagic.NewDefault()
	acme.Storage = &certmagic.FileStorage{Path: "./deploy-tool-output"}
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

// getUserCATermsAgreement retrieves whether or not the user has agreed to the CA terms
func getUserCATermsAgreement() bool {
	val := os.Getenv("D_CA_AGREE")
	if strings.ToLower(val) == "true" {
		return true
	}
	return false
}

// getUserEmail retrieves the email the user want's to associate with this certificate
func getUserEmail() (string, error) {
	val := os.Getenv("D_CA_EMAIL")
	if strings.Contains(val, "@") == false {
		return "", fmt.Errorf("invalid email address %s", val)
	}

	return val, nil
}
