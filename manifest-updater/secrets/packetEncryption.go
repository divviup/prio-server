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
	packetDecryptionKeyFormat = "packet-decryption-key-"
)

func (k *Kube) validateAndUpdatePacketEncryptionKey(secret *corev1.Secret) ([]*PrioKey, error) {
	oldPrioKey, err := NewKeyFromKubernetes(secret, PrioKeyFromX962UncompressedKey)
	if err != nil {
		return nil, fmt.Errorf("unable to create PrioKey from kubernetes secret: %v", err)
	}
	creation := secret.GetCreationTimestamp()
	since := time.Since(creation.Time)

	shouldRotate := since > k.packetEncryptionKeySpec.rotationPeriod
	if !shouldRotate {
		return []*PrioKey{oldPrioKey}, nil
	}

	k.log.
		WithField("KeyType: ", "PacketDecryptionKey").
		WithField("Should Rotate: ", shouldRotate).
		Info("Secret value didn't exist, or secret should rotate was true. we're going to assume the secret is invalid and make a new one")

	key, err := k.createAndStorePacketEncryptionKey()

	if err != nil {
		k.log.WithError(err).Errorln("Secret creation after deletion failed! This is going to cause problems :(")
		return nil, fmt.Errorf("unable to create secret after deleting: %w", err)
	}

	return []*PrioKey{
		key,
		oldPrioKey,
	}, nil
}

func (k *Kube) createAndStorePacketEncryptionKey() (*PrioKey, error) {
	key, err := NewPrioKey()

	if err != nil {
		return nil, fmt.Errorf("unable to create a packet encryption key: %w", err)
	}

	immutable := true

	secret := corev1.Secret{
		ObjectMeta: v1.ObjectMeta{
			GenerateName: packetDecryptionKeyFormat,
			Namespace:    k.namespace,
			Labels: map[string]string{
				"type": "packet-decryption-key",
			},
		},
		Immutable: &immutable,

		StringData: map[string]string{
			secretKeyMap: base64.StdEncoding.EncodeToString(key.marshallX962UncompressedPrivateKey()),
		},
	}

	sApi := k.client.CoreV1().Secrets(k.namespace)
	created, err := sApi.Create(context.Background(), &secret, v1.CreateOptions{})

	if err != nil {
		return nil, fmt.Errorf("failed to store secret: %w", err)
	}

	key.KubeIdentifier = &created.Name

	return key, err
}
