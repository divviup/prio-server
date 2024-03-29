package key

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/binary"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"math/big"
	"strings"
	"testing"
)

func TestP256(t *testing.T) {
	t.Parallel()

	key, err := P256.New()
	if err != nil {
		t.Fatalf("Couldn't create new key: %v", err)
	}
	wantPK := key.m.(*p256).privKey // grab *ecdsa.PrivateKey from guts of raw key

	// Check that each of the encodings can be round-tripped back from the
	// format it is expected to be in.
	t.Run("binary", func(t *testing.T) {
		t.Parallel()
		binaryBytes, err := key.MarshalBinary()
		if err != nil {
			t.Fatalf("Couldn't marshal to binary: %v", err)
		}

		var newKey Material
		if err := newKey.UnmarshalBinary(binaryBytes); err != nil {
			t.Fatalf("Couldn't unmarshal from binary: %v", err)
		}
		newPK := newKey.m.(*p256).privKey
		if !newPK.Equal(wantPK) {
			t.Errorf("Binary-encoded key does not match generated private key")
		}
	})

	t.Run("text", func(t *testing.T) {
		t.Parallel()
		textBytes, err := key.MarshalText()
		if err != nil {
			t.Errorf("Couldn't marshal to text: %v", err)
		}

		var newKey Material
		if err := newKey.UnmarshalText(textBytes); err != nil {
			t.Fatalf("Couldn't unmarshal from text: %v", err)
		}
		newPK := newKey.m.(*p256).privKey
		if !newPK.Equal(wantPK) {
			t.Errorf("Text-encoded key does not match generated private key")
		}
	})

	t.Run("text: too long", func(t *testing.T) {
		const (
			goodKey    = "AQJGD3XAhT4EZwSqbfj/9xnh5BmPLftXDEl0RnjwPmoqkLS8PLbNk5jo9HgIaaLEsjEthmdzQf+WC/0+Ccr5Odw3" // taken from dev environment
			badKey     = goodKey + "AA"
			wantErrStr = "wrong length"
		)

		var k Material
		err := k.UnmarshalText([]byte(badKey))
		if err == nil || !strings.Contains(err.Error(), wantErrStr) {
			t.Errorf("Wanted error containing %q, got: %v", wantErrStr, err)
		}
	})

	t.Run("Public", func(t *testing.T) {
		t.Parallel()
		gotKey := key.Public()
		if !gotKey.Equal(wantPK.Public()) {
			t.Errorf("Public key does not match generated public key")
		}
	})

	t.Run("PublicAsCSR", func(t *testing.T) {
		t.Parallel()
		const fqdn = "my.bogus.fqdn"
		pemCSRBytes, err := key.PublicAsCSR(fqdn)
		if err != nil {
			t.Fatalf("Couldn't serialize public key as CSR: %v", err)
		}

		pemCSR, rest := pem.Decode([]byte(pemCSRBytes))
		if pemCSR == nil {
			t.Fatalf("Couldn't parse as PEM: %q", pemCSR)
		}
		if len(rest) > 0 {
			t.Errorf("Extra bytes in PEM-encoding: %q", string(rest))
		}
		if wantCSRType := "CERTIFICATE REQUEST"; pemCSR.Type != wantCSRType {
			t.Errorf("PEM block got type %q, want type %q", pemCSR, wantCSRType)
		}
		if len(pemCSR.Headers) > 0 {
			t.Errorf("PEM block unexpectedly had headers: %q", pemCSR.Headers)
		}

		csr, err := x509.ParseCertificateRequest(pemCSR.Bytes)
		if err != nil {
			t.Fatalf("Couldn't parse as CSR: %v", err)
		}
		if err := csr.CheckSignature(); err != nil {
			t.Errorf("CSR not properly signed: %v", err)
		}
		wantCSRSubject := pkix.Name{CommonName: fqdn}
		if csr.Subject.String() != wantCSRSubject.String() {
			t.Errorf("CSR subject got %q, want %q", csr.Subject, wantCSRSubject)
		}
		csrPubkey, ok := csr.PublicKey.(*ecdsa.PublicKey)
		if !ok {
			t.Fatalf("CSR public key was a %T, want %T", csr.PublicKey, (*ecdsa.PublicKey)(nil))
		}
		if !csrPubkey.Equal(wantPK.Public()) {
			t.Errorf("CSR public key does not match generated public key")
		}
	})

	t.Run("PublicAsPKIX", func(t *testing.T) {
		t.Parallel()
		pemPKIXBytes, err := key.PublicAsPKIX()
		if err != nil {
			t.Fatalf("Couldn't serialize public key as PKIX: %v", err)
		}

		pemPKIX, rest := pem.Decode([]byte(pemPKIXBytes))
		if pemPKIX == nil {
			t.Fatalf("Couldn't parse as PEM: %q", pemPKIX)
		}
		if len(rest) > 0 {
			t.Errorf("Extra bytes in PEM-encoding: %q", string(rest))
		}
		if wantCSRType := "PUBLIC KEY"; pemPKIX.Type != wantCSRType {
			t.Errorf("PEM block got type %q, want type %q", pemPKIX, wantCSRType)
		}
		if len(pemPKIX.Headers) > 0 {
			t.Errorf("PEM block unexpectedly had headers: %q", pemPKIX.Headers)
		}

		pkix, err := x509.ParsePKIXPublicKey(pemPKIX.Bytes)
		if err != nil {
			t.Fatalf("Couldn't parse as PKIX: %v", err)
		}
		pkixPubkey, ok := pkix.(*ecdsa.PublicKey)
		if !ok {
			t.Fatalf("PKIX public key was a %T, want %T", pkix, (*ecdsa.PublicKey)(nil))
		}
		if !pkixPubkey.Equal(wantPK.Public()) {
			t.Errorf("PKIX public key does not match generated public key")
		}
	})

	t.Run("AsX962Uncompressed", func(t *testing.T) {
		t.Parallel()
		b64X962Bytes, err := key.AsX962Uncompressed()
		if err != nil {
			t.Fatalf("Couldn't serialize private key as X9.62: %v", err)
		}

		x962Bytes, err := base64.StdEncoding.DecodeString(b64X962Bytes)
		if err != nil {
			t.Fatalf("Couldn't base64-decode: %v", err)
		}

		const marshalledPubkeyLength = 65
		x, y := elliptic.Unmarshal(elliptic.P256(), x962Bytes[:marshalledPubkeyLength])
		d := new(big.Int).SetBytes(x962Bytes[marshalledPubkeyLength:])
		x962Key := &ecdsa.PrivateKey{
			PublicKey: ecdsa.PublicKey{
				Curve: elliptic.P256(),
				X:     x,
				Y:     y,
			},
			D: d,
		}
		if !x962Key.Equal(wantPK) {
			t.Errorf("X9.62 private key does not match generated private key")
		}
	})

	t.Run("AsPKCS8", func(t *testing.T) {
		t.Parallel()
		b64PKCS8Bytes, err := key.AsPKCS8()
		if err != nil {
			t.Fatalf("Couldn't serialize private key as PKCS #8: %v", err)
		}

		pkcs8Bytes, err := base64.StdEncoding.DecodeString(b64PKCS8Bytes)
		if err != nil {
			t.Fatalf("Couldn't base64-decode: %v", err)
		}

		pkcs8, err := x509.ParsePKCS8PrivateKey(pkcs8Bytes)
		if err != nil {
			t.Fatalf("Couldn't parse as PKCS #8 private key: %v", err)
		}
		pkcs8Key, ok := pkcs8.(*ecdsa.PrivateKey)
		if !ok {
			t.Fatalf("PKCS #8 private key was a %T, want %T", pkcs8, (*ecdsa.PrivateKey)(nil))
		}
		if !pkcs8Key.Equal(wantPK) {
			t.Fatalf("PKCS #8 private key does not match generated private key")
		}
	})

	t.Run("P256MaterialFrom", func(t *testing.T) {
		t.Parallel()

		// Success test.
		t.Run("happy path", func(t *testing.T) {
			t.Parallel()
			if _, err := P256MaterialFrom(&ecdsa.PrivateKey{
				PublicKey: ecdsa.PublicKey{
					Curve: elliptic.P256(),
					X:     mustInt("100281053943626114588339627807397740475849787919368479671799651521728988695054"),
					Y:     mustInt("92848018789799398563224167584887395252439620813688048638482994377853029146245"),
				},
				D: mustInt("8496960630434574126270397013403207859297604121831246711994989434547040199290"),
			}); err != nil {
				t.Errorf("Unexpected error from P256MaterialFrom: %v", err)
			}
		})

		// Failure tests.
		for _, test := range []struct {
			name       string
			key        *ecdsa.PrivateKey
			wantErrStr string
		}{
			{
				name:       "not a P-256 key",
				key:        mustKey(elliptic.P384()),
				wantErrStr: "rather than P-256",
			},
			{
				name: "invalid public key",
				key: &ecdsa.PrivateKey{
					PublicKey: ecdsa.PublicKey{
						Curve: elliptic.P256(),
						X:     mustInt("42"), // should be "100281053943626114588339627807397740475849787919368479671799651521728988695054" instead (obviously)
						Y:     mustInt("92848018789799398563224167584887395252439620813688048638482994377853029146245"),
					},
					D: mustInt("8496960630434574126270397013403207859297604121831246711994989434547040199290"),
				},
				wantErrStr: "invalid public key",
			},
			{
				name: "public key does not correspond to private key",
				key: &ecdsa.PrivateKey{
					PublicKey: ecdsa.PublicKey{
						Curve: elliptic.P256(),
						X:     mustInt("100281053943626114588339627807397740475849787919368479671799651521728988695054"),
						Y:     mustInt("92848018789799398563224167584887395252439620813688048638482994377853029146245"),
					},
					D: mustInt("42"), // should be "8496960630434574126270397013403207859297604121831246711994989434547040199290" instead (obviously)
				},
				wantErrStr: "key mismatch",
			},
		} {
			test := test
			t.Run(test.name, func(t *testing.T) {
				t.Parallel()
				_, err := P256MaterialFrom(test.key)
				if err == nil || !strings.Contains(err.Error(), test.wantErrStr) {
					t.Errorf("Wanted error containing %q, got: %v", test.wantErrStr, err)
				}
			})
		}
	})
}

func mustInt(digits string) *big.Int {
	var z big.Int
	if _, ok := z.SetString(digits, 10); !ok {
		panic(fmt.Sprintf("Couldn't set digits of big.Int to %q", digits))
	}
	return &z
}

func mustKey(c elliptic.Curve) *ecdsa.PrivateKey {
	k, err := ecdsa.GenerateKey(c, rand.Reader)
	if err != nil {
		panic(fmt.Sprintf("Couldn't generate %q key: %v", c.Params().Name, err))
	}
	return k
}

// For in-package testing, create a new "TEST" raw key material type; keys are
// a single int64 value, which can be chosen by the test logic by calling
// newTestKey(int64).
const Test Type = 0

func init() {
	typeInfos[Test] = &typeInfo{
		name:             "TEST",
		newRandom:        newRandomTestKey,
		newUninitialized: newUninitializedTestKey,
	}
}

type testKey struct{ privKey int64 }

var _ material = &testKey{} // verify *testKey satisfies material

func newTestKey(pk int64) Material { return Material{&testKey{pk}} }

func newRandomTestKey() (material, error) {
	var buf [8]byte
	if _, err := io.ReadFull(rand.Reader, buf[:]); err != nil {
		return nil, fmt.Errorf("couldn't read from random: %v", err)
	}
	return &testKey{(int64)(binary.BigEndian.Uint64(buf[:]))}, nil
}

func newUninitializedTestKey() material { return &testKey{} }

func (testKey) keyType() Type { return Test }

func (k testKey) equal(o material) bool { return k.privKey == o.(*testKey).privKey }

func (k testKey) public() *ecdsa.PublicKey { panic("unimplemented") }

func (k testKey) publicAsCSR(csrFQDN string) (string, error) { return "", errors.New("unimplemented") }

func (k testKey) publicAsPKIX() (string, error) { return "", errors.New("unimplemented") }

func (k testKey) asX962Uncompressed() (string, error) { return "", errors.New("unimplemented") }

func (k testKey) asPKCS8() (string, error) { return "", errors.New("unimplemented") }

func (k testKey) MarshalBinary() ([]byte, error) {
	// Test keys' raw key format is the big-endian encoding of the "private
	// key" (int64).
	var buf [8]byte
	binary.BigEndian.PutUint64(buf[:], uint64(k.privKey))
	return buf[:], nil
}

func (k *testKey) UnmarshalBinary(data []byte) error {
	if len(data) != 8 {
		return fmt.Errorf("wrong serialization length for test key (want 8, got %d)", len(data))
	}
	*k = testKey{int64(binary.BigEndian.Uint64(data))}
	return nil
}
