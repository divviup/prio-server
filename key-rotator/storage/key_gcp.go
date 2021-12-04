package storage

import (
	"context"
	"encoding/json"
	"fmt"

	secretmanager "cloud.google.com/go/secretmanager/apiv1"
	"github.com/googleapis/gax-go/v2"
	"github.com/rs/zerolog/log"
	smpb "google.golang.org/genproto/googleapis/cloud/secretmanager/v1"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

// NewGCPKey returns a Key implementation using the GCP secret manager for
// backing storage. This key store writes keys in a way that is suitable for
// backup; keys written by this store cannot be read by other components of the
// Prio system (e.g. the facilitator).
func NewGCPKey(sm *secretmanager.Client, prioEnv, gcpProjectID string) Key {
	return gcpKey{sm, prioEnv, gcpProjectID}
}

type gcpKey struct {
	sm           gcpSecretManager
	env          string
	gcpProjectID string
}

// gcpSecretManager is an internal interface used, intended to be satisfied by
// the "real" GCP secret manager client API (*secretmanager.Client). It exists
// to enable testability.
type gcpSecretManager interface {
	AccessSecretVersion(context.Context, *smpb.AccessSecretVersionRequest, ...gax.CallOption) (*smpb.AccessSecretVersionResponse, error)
	AddSecretVersion(context.Context, *smpb.AddSecretVersionRequest, ...gax.CallOption) (*smpb.SecretVersion, error)
	CreateSecret(context.Context, *smpb.CreateSecretRequest, ...gax.CallOption) (*smpb.Secret, error)
}

// verify gcpSecretManager is satisfied by the expected production implementation
var _ gcpSecretManager = (*secretmanager.Client)(nil)

func (k gcpKey) PutBatchSigningKey(ctx context.Context, locality, ingestor string, key key.Key) error {
	return k.putKey(ctx, "batch-signing", batchSigningKeyName(k.env, locality, ingestor), key)
}

func (k gcpKey) PutPacketEncryptionKey(ctx context.Context, locality string, key key.Key) error {
	return k.putKey(ctx, "packet-encryption", packetEncryptionKeyName(k.env, locality), key)
}

func (k gcpKey) putKey(ctx context.Context, secretKind, secretName string, key key.Key) error {
	log.Info().
		Str("storage", "gcp").
		Str("kind", secretKind).
		Str("secret", secretName).
		Msgf("Writing key to secret %q", secretName)

	// Serialize the key we will be writing to GCP.
	keyBytes, err := json.Marshal(key)
	if err != nil {
		return fmt.Errorf("couldn't serialize key: %w", err)
	}

	// Create the GCP secret, if it doesn't already exist.
	if _, err := k.sm.CreateSecret(ctx, &smpb.CreateSecretRequest{
		Parent:   fmt.Sprintf("projects/%s", k.gcpProjectID),
		SecretId: secretName,
		Secret: &smpb.Secret{
			Replication: &smpb.Replication{
				Replication: &smpb.Replication_Automatic_{Automatic: &smpb.Replication_Automatic{}},
			},
		},
	}); err != nil {
		// XXX: handle "already exists" error
		return fmt.Errorf("couldn't create GCP secret: %w", err)
	}

	// Add a version to the secret.
	if _, err := k.sm.AddSecretVersion(ctx, &smpb.AddSecretVersionRequest{
		Parent:  fmt.Sprintf("projects/%s/secrets/%s", k.gcpProjectID, secretName),
		Payload: &smpb.SecretPayload{Data: keyBytes},
	}); err != nil {
		return fmt.Errorf("couldn't add GCP secret version: %w", err)
	}
	return nil
}

func (k gcpKey) GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error) {
	return k.getKey(ctx, batchSigningKeyName(k.env, locality, ingestor))
}

func (k gcpKey) GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error) {
	return k.getKey(ctx, packetEncryptionKeyName(k.env, locality))
}

func (k gcpKey) getKey(ctx context.Context, secretName string) (key.Key, error) {
	sv, err := k.sm.AccessSecretVersion(ctx, &smpb.AccessSecretVersionRequest{
		Name: fmt.Sprintf("projects/%s/secrets/%s/versions/latest", k.gcpProjectID, secretName),
	})
	if err != nil {
		return key.Key{}, fmt.Errorf("couldn't retrieve secret %q: %w", secretName, err)
	}

	var secretKey key.Key
	if err := json.Unmarshal(sv.Payload.Data, &secretKey); err != nil {
		return key.Key{}, fmt.Errorf("couldn't parse key from secret %q: %w", secretName, err)
	}
	return secretKey, nil
}
