package storage

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/awserr"
	"github.com/aws/aws-sdk-go/aws/request"
	"github.com/aws/aws-sdk-go/service/secretsmanager"
	"github.com/rs/zerolog/log"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

// NewAWSKey returns a Key implementation using the AWS secret manager for
// backing storage. This key store writes keys in a way that is suitable for
// backup; keys written by this store cannot be read by other components of the
// Prio system (e.g. the facilitator).
func NewAWSKey(sm *secretsmanager.SecretsManager, prioEnv string) Key { return awsKey{sm, prioEnv} }

type awsKey struct {
	sm  awsSecretManager
	env string
}

var _ Key = awsKey{} // verify awsKey satisfies Key

// awsSecretManager is an internal interface, intended to be satisfied by the
// "real" AWS secret manager client API (*secretsmanager.SecretsManager). It
// exists to enable testability.
type awsSecretManager interface {
	CreateSecretWithContext(context.Context, *secretsmanager.CreateSecretInput, ...request.Option) (*secretsmanager.CreateSecretOutput, error)
	GetSecretValueWithContext(context.Context, *secretsmanager.GetSecretValueInput, ...request.Option) (*secretsmanager.GetSecretValueOutput, error)
	PutSecretValueWithContext(context.Context, *secretsmanager.PutSecretValueInput, ...request.Option) (*secretsmanager.PutSecretValueOutput, error)
}

// verify awsSecretManager is satisfied by the expected production implementation
var _ awsSecretManager = (*secretsmanager.SecretsManager)(nil)

func (k awsKey) PutBatchSigningKey(ctx context.Context, locality, ingestor string, key key.Key) error {
	return k.putKey(ctx, "batch-signing", batchSigningKeyName(k.env, locality, ingestor), key)
}

func (k awsKey) PutPacketEncryptionKey(ctx context.Context, locality string, key key.Key) error {
	return k.putKey(ctx, "packet-encryption", packetEncryptionKeyName(k.env, locality), key)
}

func (k awsKey) putKey(ctx context.Context, secretKind, secretName string, key key.Key) error {
	log.Info().
		Str("storage", "aws").
		Str("kind", secretKind).
		Str("secret", secretName).
		Msgf("Writing key to secret %q", secretName)

	// Serialize the key we will be writing to AWS.
	keyBytes, err := json.Marshal(key)
	if err != nil {
		return fmt.Errorf("couldn't serialize key: %w", err)
	}

	// Create the AWS secret, if it doesn't already exist.
	if _, err := k.sm.CreateSecretWithContext(ctx, &secretsmanager.CreateSecretInput{
		Name: aws.String(secretName),
	}); err != nil {
		// If the secret already exists, CreateSecret will return a ResourceExistsException.
		// Treat this error case as acceptable, and fail out on any other errors.
		if awsErr, ok := err.(awserr.Error); !ok || awsErr.Code() != secretsmanager.ErrCodeResourceExistsException {
			return fmt.Errorf("couldn't create AWS secret: %w", err)
		}
	}

	// Add a version to the secret.
	if _, err := k.sm.PutSecretValueWithContext(ctx, &secretsmanager.PutSecretValueInput{
		SecretId:     aws.String(secretName),
		SecretBinary: keyBytes,
	}); err != nil {
		return fmt.Errorf("couldn't add AWS secret version: %w", err)
	}
	return nil
}

func (k awsKey) GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error) {
	return k.getKey(ctx, batchSigningKeyName(k.env, locality, ingestor))
}

func (k awsKey) GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error) {
	return k.getKey(ctx, packetEncryptionKeyName(k.env, locality))
}

func (k awsKey) getKey(ctx context.Context, secretName string) (key.Key, error) {
	out, err := k.sm.GetSecretValueWithContext(ctx, &secretsmanager.GetSecretValueInput{
		SecretId: aws.String(secretName),
	})
	if err != nil {
		return key.Key{}, fmt.Errorf("couldn't retrieve secret %q: %w", secretName, err)
	}

	var secretKey key.Key
	if err := json.Unmarshal(out.SecretBinary, &secretKey); err != nil {
		return key.Key{}, fmt.Errorf("couldn't parse key from secret %q: %w", secretName, err)
	}
	return secretKey, nil
}
