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
	"math/big"
)

// Type represents the kind of key represented by a key.Material.
type Type uint8

const (
	// P256 represents an ECDSA P-256 key.
	P256 Type = 1 + iota
)

type typeInfo struct {
	name             string                   // string name of type
	newRandom        func() (material, error) // function returning a newly-initialized random key
	newUninitialized func() material          // function returning an uninitialized key of this type, e.g. for use in unmarshalling
}

var typeInfos = map[Type]*typeInfo{
	P256: {"P256", newRandomP256, newUninitializedP256},
}

func (t Type) String() string {
	if ti := typeInfos[t]; ti != nil {
		return ti.name
	}
	return "UNKNOWN"
}

// New creates a new, randomly-initialized key.
func (t Type) New() (Material, error) {
	ti := typeInfos[t]
	if ti == nil {
		return Material{}, fmt.Errorf("unknown key type %v (%d)", t, t)
	}
	m, err := ti.newRandom()
	if err != nil {
		return Material{}, fmt.Errorf("couldn't create %v key: %v", t, err)
	}
	return Material{m}, nil
}

// Material represents raw key material for an asymmetric cryptographic key,
// including both the private & public portions. It has functionality related
// to serialization of the key.
type Material struct{ m material }

// Verify that Material satisfies the binary/text (Un)marshaler interfaces.
var _ encoding.BinaryMarshaler = Material{}
var _ encoding.BinaryUnmarshaler = &Material{}
var _ encoding.TextMarshaler = Material{}
var _ encoding.TextUnmarshaler = &Material{}

func (m Material) MarshalBinary() ([]byte, error) {
	kBytes, err := m.m.MarshalBinary()
	if err != nil {
		return nil, fmt.Errorf("couldn't serialize %v key: %w", m.Type(), err)
	}
	return append([]byte{byte(m.Type())}, kBytes...), nil
}

func (m *Material) UnmarshalBinary(data []byte) error {
	if len(data) == 0 {
		return errors.New("empty input")
	}

	t := Type(data[0])
	ti := typeInfos[t]
	if ti == nil {
		return fmt.Errorf("unknown key type %v (%d)", t, t)
	}
	newM := ti.newUninitialized()
	if err := newM.UnmarshalBinary(data[1:]); err != nil {
		return fmt.Errorf("couldn't unmarshal %v key: %w", t, err)
	}
	*m = Material{newM}
	return nil
}

func (m Material) MarshalText() ([]byte, error) {
	binBytes, err := m.MarshalBinary()
	if err != nil {
		return nil, err
	}
	buf := make([]byte, base64.RawStdEncoding.EncodedLen(len(binBytes)))
	base64.RawStdEncoding.Encode(buf, binBytes)
	return buf, nil
}

func (m *Material) UnmarshalText(data []byte) error {
	binBytes := make([]byte, base64.RawStdEncoding.DecodedLen(len(data)))
	if _, err := base64.RawStdEncoding.Decode(binBytes, data); err != nil {
		return fmt.Errorf("couldn't decode base64: %v", err)
	}
	return m.UnmarshalBinary(binBytes)
}

func (m Material) Equal(o Material) bool {
	if m.Type() != o.Type() {
		return false
	}
	return m.m.equal(o.m)
}

// Type returns the type of the key material.
func (m Material) Type() Type { return m.m.keyType() }

// PublicAsCSR returns a PEM-encoding of the ASN.1 DER-encoding of a PKCS#10
// (RFC 2986) CSR over the public portion of the key, signed using the private
// portion of the key, using the provided FQDN as the common name for the
// request.
func (m Material) PublicAsCSR(csrFQDN string) (string, error) { return m.m.publicAsCSR(csrFQDN) }

// PublicAsPKIX returns a PEM-encoding of the ASN.1 DER-encoding of the
// public portion of the key in PKIX (RFC 5280) format.
func (m Material) PublicAsPKIX() (string, error) { return m.m.publicAsPKIX() }

// AsX962Uncompressed returns a base64 encoding of the X9.62 uncompressed
// encoding of the public portion of the key, concatenated with the secret
// "D" scalar.
func (m Material) AsX962Uncompressed() (string, error) { return m.m.asX962Uncompressed() }

// AsPKCS8 returns a base64 encoding of the ASN.1 DER-encoding of the key
// in PKCS#8 (RFC 5208) format.
func (m Material) AsPKCS8() (string, error) { return m.m.asPKCS8() }

// material represents key material of one particular type.
type material interface {
	encoding.BinaryMarshaler
	encoding.BinaryUnmarshaler

	// keyType returns the type of the key material.
	keyType() Type

	// equal determines if this key material is equal to the given key
	// material, which can be assumed to be of the same key type.
	equal(o material) bool

	// publicAsCSR returns a PEM-encoding of the ASN.1 DER-encoding of a
	// PKCS#10 (RFC 2986) CSR over the public portion of the key, signed using
	// the private portion of the key, using the provided FQDN as the common
	// name for the request.
	publicAsCSR(csrFQDN string) (string, error)

	// publicAsPKIX returns a PEM-encoding of the ASN.1 DER-encoding of the
	// public portion of the key in PKIX (RFC 5280) format.
	publicAsPKIX() (string, error)

	// asX962Uncompressed returns a base64 encoding of the X9.62 uncompressed
	// encoding of the public portion of the key, concatenated with the secret
	// "D" scalar.
	asX962Uncompressed() (string, error)

	// asPKCS8 returns a base64 encoding of the ASN.1 DER-encoding of the key
	// in PKCS#8 (RFC 5208) format.
	asPKCS8() (string, error)
}

type p256 struct{ privKey *ecdsa.PrivateKey }

var _ material = &p256{} // verify p256 implements material

// P256From returns a new Material of type P256 based on the given P256 private
// key.
func P256MaterialFrom(key *ecdsa.PrivateKey) (Material, error) {
	if key.Curve != elliptic.P256() {
		return Material{}, fmt.Errorf("key was %s rather than P256", key.Curve.Params().Name)
	}
	return Material{&p256{key}}, nil
}

func newRandomP256() (material, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("couldn't generate new key: %w", err)
	}
	return &p256{key}, nil
}

func newUninitializedP256() material { return &p256{} }

func (p256) keyType() Type { return P256 }

func (m p256) equal(o material) bool { return m.privKey.Equal(o.(*p256).privKey) }

func (m p256) publicAsCSR(csrFQDN string) (string, error) {
	tmpl := &x509.CertificateRequest{
		SignatureAlgorithm: x509.ECDSAWithSHA256,
		Subject:            pkix.Name{CommonName: csrFQDN},
	}
	csrBytes, err := x509.CreateCertificateRequest(rand.Reader, tmpl, m.privKey)
	if err != nil {
		return "", fmt.Errorf("couldn't create certificate request: %w", err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: csrBytes})), nil
}

func (m p256) publicAsPKIX() (string, error) {
	pubkeyBytes, err := x509.MarshalPKIXPublicKey(m.privKey.Public())
	if err != nil {
		return "", fmt.Errorf("couldn't encode as PKIX: %w", err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: pubkeyBytes})), nil
}

func (m p256) asX962Uncompressed() (string, error) {
	return base64.StdEncoding.EncodeToString(append(elliptic.Marshal(elliptic.P256(), m.privKey.X, m.privKey.Y), m.privKey.D.Bytes()...)), nil
}

func (m p256) asPKCS8() (string, error) {
	keyBytes, err := x509.MarshalPKCS8PrivateKey(m.privKey)
	if err != nil {
		return "", fmt.Errorf("couldn't encode as PKCS#8: %w", err)
	}
	return base64.StdEncoding.EncodeToString(keyBytes), nil
}

func (m p256) MarshalBinary() ([]byte, error) {
	// P256's raw key format is the X9.62 compressed encoding of the public
	// portion of the key, concatenated with the secret "D" scalar.
	return append(elliptic.MarshalCompressed(elliptic.P256(), m.privKey.X, m.privKey.Y), m.privKey.D.Bytes()...), nil
}

func (m *p256) UnmarshalBinary(data []byte) error {
	const p256PubkeyCompressedLen = 33 // P256 uses 256-bit = 8-byte points; the length of MarshalCompressed is 1 + point_byte_length.

	x, y := elliptic.UnmarshalCompressed(elliptic.P256(), data[:p256PubkeyCompressedLen])
	if x == nil {
		return errors.New("couldn't unmarshal compressed public key")
	}
	d := new(big.Int).SetBytes(data[p256PubkeyCompressedLen:])

	*m = p256{&ecdsa.PrivateKey{
		PublicKey: ecdsa.PublicKey{
			Curve: elliptic.P256(),
			X:     x,
			Y:     y,
		},
		D: d,
	}}
	return nil
}
