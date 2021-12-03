package storage

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"math/big"

	"github.com/rs/zerolog/log"
	k8smeta "k8s.io/apimachinery/pkg/apis/meta/v1"
	k8s "k8s.io/client-go/kubernetes/typed/core/v1"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

// NewKubernetesKey returns a new Key implementation, using the given
// Kubernetes secret interface for backing storage. This key store writes keys
// in a way that can be read by other components of the system (e.g. the
// facilitator).
func NewKubernetesKey(k8s k8s.SecretInterface, prioEnv string) (Key, error) {
	return k8sKey{k8s, prioEnv}, nil
}

type k8sKey struct {
	k8s k8s.SecretInterface
	env string // Prio environment name, e.g. "prod-us" or "prod-intl".
}

const (
	liveVersionsSecretKey = "secret_key"
	keyVersionsSecretKey  = "key_versions"
	primaryKIDSecretKey   = "primary_kid"

	secretKeyUnfilledValue = "not-a-real-key" // used in the secret_key secret key to denote no data
)

var _ Key = k8sKey{} // verify k8skey satisfies Key

func (k k8sKey) PutBatchSigningKey(ctx context.Context, locality, ingestor string, key key.Key) error {
	return k.putKey(ctx, "batch-signing", k.batchSigningKeyName(locality, ingestor), key, serializeBatchSigningSecretKey)
}

func (k k8sKey) PutPacketEncryptionKey(ctx context.Context, locality string, key key.Key) error {
	return k.putKey(ctx, "packet-encryption", k.packetEncryptionKeyName(locality), key, serializePacketEncryptionSecretKey)
}

func (k k8sKey) putKey(ctx context.Context, secretKind, secretName string, key key.Key, serializeLiveVersions func(key.Key) ([]byte, error)) error {
	log.Info().
		Str("storage", "kubernetes").
		Str("kind", secretKind).
		Str("secret", secretName).
		Msgf("Writing key to secret %q", secretName)

	// Serialize data to be included in secret.
	keyVersionsBytes, err := json.Marshal(key)
	if err != nil {
		return fmt.Errorf("couldn't serialize key versions: %w", err)
	}
	liveVersionsBytes, err := serializeLiveVersions(key)
	if err != nil {
		return fmt.Errorf("couldn't serialize secret key: %w", err)
	}
	primaryKID := primaryKID(secretName, key)
	secretData := map[string][]byte{
		keyVersionsSecretKey:  keyVersionsBytes,
		liveVersionsSecretKey: liveVersionsBytes,
		primaryKIDSecretKey:   []byte(primaryKID),
	}

	// Write update back to Kubernetes secret store.
	s, err := k.k8s.Get(ctx, secretName, k8smeta.GetOptions{})
	if err != nil {
		return fmt.Errorf("couldn't get secret %q: %w", secretName, err)
	}
	s.Data = secretData
	if _, err := k.k8s.Update(ctx, s, k8smeta.UpdateOptions{}); err != nil {
		return fmt.Errorf("couldn't update secret %q: %w", secretName, err)
	}
	return nil
}

func (k k8sKey) GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error) {
	return k.getKey(ctx, k.batchSigningKeyName(locality, ingestor), parseBatchSigningSecretKey)
}

func (k k8sKey) GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error) {
	return k.getKey(ctx, k.packetEncryptionKeyName(locality), parsePacketEncryptionSecretKey)
}

func (k k8sKey) getKey(ctx context.Context, secretName string, parseSecretKey func([]byte) (key.Material, error)) (key.Key, error) {
	s, err := k.k8s.Get(ctx, secretName, k8smeta.GetOptions{})
	if err != nil {
		return key.Key{}, fmt.Errorf("couldn't retrieve secret %q: %w", secretName, err)
	}

	// Parse as a "new" key_versions-serialized key.
	if keyVersions, ok := s.Data[keyVersionsSecretKey]; ok {
		var secretKey key.Key
		if err := json.Unmarshal(keyVersions, &secretKey); err != nil {
			return key.Key{}, fmt.Errorf("couldn't parse key versions from secret %q: %w", secretName, err)
		}
		return secretKey, nil
	}

	// Parse as an "old" secret_key-serialized key.
	if liveVersion, ok := s.Data[liveVersionsSecretKey]; ok && string(liveVersion) != secretKeyUnfilledValue {
		keyMaterialBytes := make([]byte, base64.StdEncoding.DecodedLen(len(liveVersion)))
		n, err := base64.StdEncoding.Decode(keyMaterialBytes, liveVersion)
		if err != nil {
			return key.Key{}, fmt.Errorf("couldn't interpret secret %q secret key as base64: %w", secretName, err)
		}
		keyMaterial, err := parseSecretKey(keyMaterialBytes[:n])
		if err != nil {
			return key.Key{}, fmt.Errorf("couldn't interpret secret %q secret key as key: %w", secretName, err)
		}
		secretKey, err := key.FromVersions(key.Version{KeyMaterial: keyMaterial, CreationTimestamp: 0})
		if err != nil {
			return key.Key{}, fmt.Errorf("couldn't create key: %w", err)
		}
		return secretKey, nil
	}

	return key.Key{}, nil
}

func (k k8sKey) batchSigningKeyName(locality, ingestor string) string {
	return fmt.Sprintf("%s-%s-%s-batch-signing-key", k.env, locality, ingestor)
}

func (k k8sKey) packetEncryptionKeyName(locality string) string {
	return fmt.Sprintf("%s-%s-ingestion-packet-decryption-key", k.env, locality)
}

func primaryKID(secretName string, key key.Key) string {
	if key.IsEmpty() || key.Primary().CreationTimestamp == 0 {
		return secretName
	}
	return fmt.Sprintf("%s-%d", secretName, key.Primary().CreationTimestamp)
}

func serializeBatchSigningSecretKey(k key.Key) ([]byte, error) {
	primaryKeyMaterial := k.Primary().KeyMaterial
	kmBytes, err := primaryKeyMaterial.AsPKCS8()
	if err != nil {
		return nil, err
	}
	return []byte(kmBytes), nil
}

func parseBatchSigningSecretKey(keyMaterialBytes []byte) (key.Material, error) {
	privKey, err := x509.ParsePKCS8PrivateKey(keyMaterialBytes)
	if err != nil {
		return key.Material{}, fmt.Errorf("couldn't interpret key material as PKCS#8: %w", err)
	}
	ecdsaPrivKey, ok := privKey.(*ecdsa.PrivateKey)
	if !ok {
		return key.Material{}, fmt.Errorf("couldn't interpret key material as ECDSA key (was %T)", privKey)
	}
	keyMaterial, err := key.P256MaterialFrom(ecdsaPrivKey)
	if err != nil {
		return key.Material{}, fmt.Errorf("couldn't interpret key material as P-256 ECDSA key: %w", err)
	}
	return keyMaterial, nil
}

func serializePacketEncryptionSecretKey(k key.Key) ([]byte, error) {
	var buf bytes.Buffer
	if err := k.Versions(func(v key.Version) error {
		if buf.Len() > 0 {
			buf.WriteRune(',')
		}
		kmBytes, err := v.KeyMaterial.AsX962Uncompressed()
		if err != nil {
			return fmt.Errorf("couldn't serialize key version: %w", err)
		}
		buf.WriteString(kmBytes)
		return nil
	}); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func parsePacketEncryptionSecretKey(keyMaterialBytes []byte) (key.Material, error) {
	const marshalledPubkeyLength = 65 // P256 uses 256 bit = 32 byte points. elliptic.Marshal produces results of 1 + 2*sizeof(point) = 65 bytes in length.
	x, y := elliptic.Unmarshal(elliptic.P256(), keyMaterialBytes[:marshalledPubkeyLength])
	if x == nil {
		return key.Material{}, errors.New("couldn't unmarshal as X9.62 public key")
	}
	d := new(big.Int).SetBytes(keyMaterialBytes[marshalledPubkeyLength:])
	return key.P256MaterialFrom(&ecdsa.PrivateKey{
		PublicKey: ecdsa.PublicKey{
			Curve: elliptic.P256(),
			X:     x,
			Y:     y,
		},
		D: d,
	})
}
