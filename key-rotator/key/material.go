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
		return Material{}, fmt.Errorf("couldn't create %v key: %w", t, err)
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
	n, err := base64.RawStdEncoding.Decode(binBytes, data)
	if err != nil {
		return fmt.Errorf("couldn't decode base64: %w", err)
	}
	return m.UnmarshalBinary(binBytes[:n])
}

func (m Material) Equal(o Material) bool {
	if m.Type() != o.Type() {
		return false
	}
	return m.m.equal(o.m)
}

// Type returns the type of the key material.
func (m Material) Type() Type { return m.m.keyType() }

// Public returns the public key associated with this key material as an
// ecdsa.PublicKey.
func (m Material) Public() *ecdsa.PublicKey { return m.m.public() }

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

	// public returns the public key associated with this key material as an
	// *ecdsa.PublicKey.
	public() *ecdsa.PublicKey

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

const (
	// P256 uses 256-bit = 32 byte points, and 256-bit = 32 byte private key (D) values.
	p256PubkeyUncompressedLen = 65 // elliptic.Marshal produces results of 1 + 2*sizeof(point) = 65 bytes in length.
	p256PubkeyCompressedLen   = 33 // elliptic.MarshalCompressed produces results of 1 + sizeof(point) = 33 bytes in length.
	p256PrivateKeyLen         = 32
)

var _ material = &p256{} // verify p256 implements material

// P256From returns a new Material of type P256 based on the given P256 private
// key.
func P256MaterialFrom(key *ecdsa.PrivateKey) (Material, error) {
	var m p256
	if err := m.setKey(key); err != nil {
		return Material{}, err
	}
	return Material{&m}, nil
}

func newRandomP256() (material, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("couldn't generate new key: %w", err)
	}
	var m p256
	if err := m.setKey(key); err != nil {
		return nil, err
	}
	return &m, nil
}

func newUninitializedP256() material { return &p256{} }

func (p256) keyType() Type { return P256 }

func (m p256) equal(o material) bool { return m.privKey.Equal(o.(*p256).privKey) }

func (m p256) public() *ecdsa.PublicKey { return &m.privKey.PublicKey }

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
	var keyBytes [p256PubkeyUncompressedLen + p256PrivateKeyLen]byte
	pubkeyBytes := elliptic.Marshal(elliptic.P256(), m.privKey.PublicKey.X, m.privKey.PublicKey.Y)
	if len(pubkeyBytes) != p256PubkeyUncompressedLen {
		panic(fmt.Sprintf("Unexpected length from elliptic.Marshal: wanted %d, got %d", p256PubkeyUncompressedLen, len(pubkeyBytes)))
	}
	copy(keyBytes[:p256PubkeyUncompressedLen], pubkeyBytes)
	m.privKey.D.FillBytes(keyBytes[p256PubkeyUncompressedLen:])
	return base64.StdEncoding.EncodeToString(keyBytes[:]), nil
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
	var keyBytes [p256PubkeyCompressedLen + p256PrivateKeyLen]byte
	pubkeyBytes := elliptic.MarshalCompressed(elliptic.P256(), m.privKey.PublicKey.X, m.privKey.PublicKey.Y)
	if len(pubkeyBytes) != p256PubkeyCompressedLen {
		panic(fmt.Sprintf("Unexpected length from elliptic.MarshalCompressed: wanted %d, got %d", p256PubkeyCompressedLen, len(pubkeyBytes)))
	}
	copy(keyBytes[:p256PubkeyCompressedLen], pubkeyBytes)
	m.privKey.D.FillBytes(keyBytes[p256PubkeyCompressedLen:])
	return keyBytes[:], nil
}

func (m *p256) UnmarshalBinary(data []byte) error {
	// Deserialize bytes back into X/Y/D values.
	if wantLen := p256PubkeyCompressedLen + p256PrivateKeyLen; len(data) != wantLen {
		return fmt.Errorf("serialized data has wrong length (want %d, got %d)", wantLen, len(data))
	}
	c := elliptic.P256()
	x, y := elliptic.UnmarshalCompressed(c, data[:p256PubkeyCompressedLen])
	if x == nil {
		return errors.New("couldn't unmarshal compressed public key")
	}
	d := new(big.Int).SetBytes(data[p256PubkeyCompressedLen:])

	return m.setKey(&ecdsa.PrivateKey{
		PublicKey: ecdsa.PublicKey{
			Curve: c,
			X:     x,
			Y:     y,
		},
		D: d,
	})
}

func (m *p256) setKey(k *ecdsa.PrivateKey) error {
	// Check that the provided key is actually a P-256 key.
	c := elliptic.P256()
	if k.Curve != c {
		return fmt.Errorf("key was %s rather than P-256", k.Curve.Params().Name)
	}

	// Check that (public key) X, Y corresponds to a point on the curve.
	// See implementation of `elliptic.Unmarshal` for evidence that this is a
	// sufficient check on the public key.
	p := c.Params().P
	if k.X.Cmp(p) >= 0 || k.Y.Cmp(p) >= 0 || !c.IsOnCurve(k.X, k.Y) {
		return errors.New("invalid public key")
	}

	// Check that (private key) D corresponds to (public key) X, Y.
	// See implementation of `ecdsa.GenerateKey` for evidence that this is a
	// correct method of translating D -> (X, Y).
	wantX, wantY := c.ScalarBaseMult(k.D.Bytes())
	if wantX.Cmp(k.X) != 0 || wantY.Cmp(k.Y) != 0 {
		return errors.New("public/private key mismatch")
	}

	*m = p256{k}
	return nil
}
