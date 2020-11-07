package secrets

import (
	"context"
	"encoding/base64"
	"fmt"
	corev1 "k8s.io/api/core/v1"
	v1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"time"
)

const (
	batchSigningKeyExpirationDuration = 90 * 24 * time.Hour

	batchSigningKeyMaxAge = 20 * time.Second
	expirationKeyMap      = "expiration"
)

func (k *Kube) validateAndUpdateBatchSigningKey(keyName, dsp string, secret *corev1.Secret) ([]*PrioKey, error) {
	creation := secret.GetCreationTimestamp()
	since := time.Since(creation.Time)

	expired := since > batchSigningKeyMaxAge

	if !expired {
		return nil, nil
	}

	k.log.
		WithField("KeyType", "BatchSigningKey").
		WithField("Expiration: ", expired).
		Info("Secret is close to expiration or has expired, we're going to require it to be expired")

	key, err := k.createAndStoreBatchSigningKey(keyName, dsp)

	if err != nil {
		return nil, fmt.Errorf("unable to create secret: %w", err)
	}

	oldExpiration := string(secret.Data[expirationKeyMap])

	encodedKey := secret.Data[secretKeyMap]
	oldKey := make([]byte, base64.StdEncoding.DecodedLen(len(encodedKey)))
	_, err = base64.StdEncoding.Decode(oldKey, secret.Data[secretKeyMap])
	if err != nil {
		return nil, fmt.Errorf("unable to decode old secret key: %w", err)
	}

	oldPrioKey := PrioKeyFromX962UncompressedKey(oldKey)
	oldPrioKey.KubeIdentifier = &secret.Name
	oldPrioKey.Expiration = &oldExpiration

	return []*PrioKey{
		key,
		&oldPrioKey,
	}, nil
}

func (k *Kube) createAndStoreBatchSigningKey(name, dsp string) (*PrioKey, error) {
	key, err := NewPrioKey()

	if err != nil {
		return nil, fmt.Errorf("unable to create a batch signing key: %w", err)
	}

	immutable := true

	expiration := time.
		Now().
		Add(batchSigningKeyExpirationDuration).
		UTC().
		Format(time.RFC3339)

	secret := corev1.Secret{
		ObjectMeta: v1.ObjectMeta{
			GenerateName: name,
			Namespace:    k.namespace,
			Labels: map[string]string{
				"type": "batch-signing-key",
				"dsp":  dsp,
			},
		},
		Immutable: &immutable,

		StringData: map[string]string{
			secretKeyMap:     base64.StdEncoding.EncodeToString(key.marshallX962UncompressedPrivateKey()),
			expirationKeyMap: expiration,
		},
	}

	sApi := k.client.CoreV1().Secrets(k.namespace)
	created, err := sApi.Create(context.Background(), &secret, v1.CreateOptions{})

	if err != nil {
		return nil, fmt.Errorf("failed to store secret %w", err)
	}

	key.KubeIdentifier = &created.Name
	key.Expiration = &expiration
	return key, nil
}
