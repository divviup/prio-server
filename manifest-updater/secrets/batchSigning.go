package secrets

import (
	"context"
	"encoding/base64"
	"fmt"
	"time"

	corev1 "k8s.io/api/core/v1"
	v1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

const (
	expirationKeyMap = "expiration"
)

func (k *Kube) validateAndUpdateBatchSigningKey(keyName, ingestor string, secret *corev1.Secret) ([]*PrioKey, error) {
	oldPrioKey, err := NewKeyFromKubernetes(secret, PrioKeyFromPKCS8PrivateKey)
	if err != nil {
		return nil, fmt.Errorf("unable to create PrioKey from kubernetes secret: %v", err)
	}
	oldExpiration := string(secret.Data[expirationKeyMap])
	oldPrioKey.Expiration = &oldExpiration

	creation := secret.GetCreationTimestamp()
	since := time.Since(creation.Time)

	shouldRotate := since > k.batchSigningKeySpec.rotationPeriod

	if !shouldRotate {
		return []*PrioKey{
			oldPrioKey,
		}, nil
	}

	k.log.
		WithField("KeyType", "BatchSigningKey").
		WithField("Should Rotate: ", shouldRotate).
		Info("Secret is close to expiration, we're going to require it to be rotated")

	key, err := k.createAndStoreBatchSigningKey(keyName, ingestor)

	if err != nil {
		return nil, fmt.Errorf("unable to create secret: %w", err)
	}

	return []*PrioKey{
		key,
		oldPrioKey,
	}, nil
}

func (k *Kube) createAndStoreBatchSigningKey(name, ingestor string) (*PrioKey, error) {
	key, err := NewPrioKey()

	if err != nil {
		return nil, fmt.Errorf("unable to create a batch signing key: %w", err)
	}

	pkcs8Key, err := key.marshalPKCS8PrivateKey()

	if err != nil {
		return nil, fmt.Errorf("unable to marshall pkcs8 key: %w", err)
	}

	immutable := true

	expiration := time.
		Now().
		Add(k.batchSigningKeySpec.expirationPeriod).
		UTC().
		Format(time.RFC3339)

	secret := corev1.Secret{
		ObjectMeta: v1.ObjectMeta{
			GenerateName: name,
			Namespace:    k.namespace,
			Labels: map[string]string{
				"type":     "batch-signing-key",
				"ingestor": ingestor,
			},
		},
		Immutable: &immutable,

		StringData: map[string]string{
			secretKeyMap:     base64.StdEncoding.EncodeToString(pkcs8Key),
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
