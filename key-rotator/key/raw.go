package key

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding"
	"encoding/base64"
	"encoding/pem"
	"errors"
	"fmt"
)

// Type represents the type of a Raw key.
type Type uint8

const (
	P256 Type = 1 + iota
)

func (t Type) String() string {
	switch t {
	case P256:
		return "P256"
	default:
		return "UNKNOWN"
	}
}

// Raw represents a raw (i.e. unversioned) asymmetric cryptographic key,
// including both the private & public portions. It has functionality related
// to serialization of the key.
type Raw struct{ k raw }

var _ encoding.BinaryMarshaler = Raw{}
var _ encoding.BinaryUnmarshaler = &Raw{}
var _ encoding.TextMarshaler = Raw{}
var _ encoding.TextUnmarshaler = &Raw{}

func NewRaw(typ Type) (Raw, error) {
	switch typ {
	case P256:
		return newP256()
	default:
		return Raw{}, fmt.Errorf("unknown key type %v (%d)", typ, typ)
	}
}

func (r Raw) MarshalBinary() ([]byte, error) {
	kBytes, err := r.k.MarshalBinary()
	if err != nil {
		return nil, fmt.Errorf("couldn't serialize %v key: %w", r.Type(), err)
	}
	return append([]byte{byte(r.Type())}, kBytes...), nil
}

func (r *Raw) UnmarshalBinary(data []byte) error {
	if len(data) == 0 {
		return errors.New("empty input")
	}

	typ := Type(data[0])
	var k raw
	switch typ := Type(data[0]); typ {
	case P256:
		k = &p256{}
	default:
		return fmt.Errorf("unknown key type %v (%d)", typ, typ)
	}
	if err := k.UnmarshalBinary(data[1:]); err != nil {
		return fmt.Errorf("couldn't unmarshal %v key: %w", typ, err)
	}
	*r = Raw{k}
	return nil
}

func (r Raw) MarshalText() ([]byte, error) {
	binBytes, err := r.MarshalBinary()
	if err != nil {
		return nil, err
	}
	buf := make([]byte, base64.RawStdEncoding.EncodedLen(len(binBytes)))
	base64.RawStdEncoding.Encode(buf, binBytes)
	return buf, nil
}

func (r *Raw) UnmarshalText(data []byte) error {
	binBytes := make([]byte, base64.RawStdEncoding.DecodedLen(len(data)))
	if _, err := base64.RawStdEncoding.Decode(binBytes, data); err != nil {
		return fmt.Errorf("couldn't decode base64: %v", err)
	}
	return r.UnmarshalBinary(binBytes)
}

// Type returns the type of the raw key.
func (r Raw) Type() Type { return r.k.Type() }

// PublicAsCSR returns a PEM-encoding of the ASN.1 DER-encoding of a
// PKCS#10 (RFC 2986) CSR over the public portion of the key, using the
// provided FQDN as the common name for the request.
func (r Raw) PublicAsCSR(csrFQDN string) (string, error) { return r.k.PublicAsCSR(csrFQDN) }

// PublicAsPKIX returns a PEM-encoding of the ASN.1 DER-encoding of the
// public portion of the key in PKIX (RFC 5280) format.
func (r Raw) PublicAsPKIX() (string, error) { return r.k.PublicAsPKIX() }

// AsX962Uncompressed returns a base64 encoding of the X9.62 uncompressed
// encoding of the public portion of the key, concatenated with the secret
// "D" scalar.
func (r Raw) AsX962Uncompressed() (string, error) { return r.k.AsX962Uncompressed() }

// AsPKCS8 returns a base64 encoding of the ASN.1 DER-encoding of the key
// in PKCS#8 (RFC 5208) format.
func (r Raw) AsPKCS8() (string, error) { return r.k.AsPKCS8() }

// raw represents a raw key of one particular type.
type raw interface {
	encoding.BinaryMarshaler
	encoding.BinaryUnmarshaler

	// Type returns the type of the raw key.
	Type() Type

	// PublicAsCSR returns a PEM-encoding of the ASN.1 DER-encoding of a
	// PKCS#10 (RFC 2986) CSR over the public portion of the key, using the
	// provided FQDN as the common name for the request.
	PublicAsCSR(csrFQDN string) (string, error)

	// PublicAsPKIX returns a PEM-encoding of the ASN.1 DER-encoding of the
	// public portion of the key in PKIX (RFC 5280) format.
	PublicAsPKIX() (string, error)

	// AsX962Uncompressed returns a base64 encoding of the X9.62 uncompressed
	// encoding of the public portion of the key, concatenated with the secret
	// "D" scalar.
	AsX962Uncompressed() (string, error)

	// AsPKCS8 returns a base64 encoding of the ASN.1 DER-encoding of the key
	// in PKCS#8 (RFC 5208) format.
	AsPKCS8() (string, error)
}

type p256 struct{ pk *ecdsa.PrivateKey }

var _ raw = &p256{} // verify p256 implements raw

func newP256() (Raw, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return Raw{}, fmt.Errorf("couldn't generate new key: %w", err)
	}
	return Raw{&p256{key}}, nil
}

func (p256) Type() Type { return P256 }

func (k p256) PublicAsCSR(csrFQDN string) (string, error) {
	tmpl := &x509.CertificateRequest{
		SignatureAlgorithm: x509.ECDSAWithSHA256,
		Subject:            pkix.Name{CommonName: csrFQDN},
	}
	csrBytes, err := x509.CreateCertificateRequest(rand.Reader, tmpl, k.pk)
	if err != nil {
		return "", fmt.Errorf("couldn't create certificate request: %w", err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: csrBytes})), nil
}

func (k p256) PublicAsPKIX() (string, error) {
	pubkeyBytes, err := x509.MarshalPKIXPublicKey(k.pk.Public())
	if err != nil {
		return "", fmt.Errorf("couldn't encode as PKIX: %w", err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: pubkeyBytes})), nil
}

func (k p256) AsX962Uncompressed() (string, error) {
	return base64.StdEncoding.EncodeToString(append(elliptic.Marshal(elliptic.P256(), k.pk.X, k.pk.Y), k.pk.D.Bytes()...)), nil
}

func (k p256) AsPKCS8() (string, error) {
	keyBytes, err := x509.MarshalPKCS8PrivateKey(k.pk)
	if err != nil {
		return "", fmt.Errorf("couldn't encode as PKCS#8: %w", err)
	}
	return base64.StdEncoding.EncodeToString(keyBytes), nil
}

func (k p256) MarshalBinary() ([]byte, error) {
	// P256's raw key format is the ASN.1 DER-encoding of the key as an RFC
	// 5915 Elliptic Curve Private Key Structure.
	return x509.MarshalECPrivateKey(k.pk)
}

func (k *p256) UnmarshalBinary(data []byte) error {
	pk, err := x509.ParseECPrivateKey(data)
	if err != nil {
		return fmt.Errorf("couldn't parse EC key structure: %w", err)
	}
	if pk.Curve != elliptic.P256() {
		return fmt.Errorf("parsed key was not a P256 key (was %q)", pk.Params().Name)
	}
	*k = p256{pk}
	return nil
}
