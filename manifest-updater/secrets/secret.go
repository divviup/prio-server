package secrets

import (
	"context"
	"fmt"
	"os"
	"sort"
	"time"

	log "github.com/sirupsen/logrus"
	corev1 "k8s.io/api/core/v1"
	v1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
)

type Kube struct {
	log       *log.Entry
	client    *kubernetes.Clientset
	namespace string
	ingestors []string

	batchSigningKeySpec     *KeySpec
	packetEncryptionKeySpec *KeySpec
}

type KeySpec struct {
	expirationPeriod time.Duration
	rotationPeriod   time.Duration
}

const (
	secretKeyMap          = "secret_key"
	batchSigningKeyFormat = "batch-signing-key-%s-"
	// We're pretty confident if we keep the last 4 secrets around we
	// should be able to decrypt old-enough data
	maxSecrets = 4
)

func NewKeySpec(expirationDays, rotationDays time.Duration) KeySpec {
	return KeySpec{
		expirationPeriod: expirationDays * 24 * time.Hour,
		rotationPeriod:   rotationDays * 24 * time.Hour,
	}
}

func NewKube(namespace string, ingestors []string, batchSigningKeySpec, packetEncryptionKeySpec KeySpec) (*Kube, error) {
	config, err := rest.InClusterConfig()
	if err != nil {
		config, err = clientcmd.BuildConfigFromFlags("", os.Getenv("KUBECONFIG"))
		if err != nil {
			return nil, fmt.Errorf("failed to get KUBECONFIG environment variable: %w", err)
		}
	}

	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, fmt.Errorf("getting the client failed: %w", err)
	}

	return &Kube{
		log:                     log.WithField("source", "kube secret").WithField("namespace", namespace),
		client:                  client,
		namespace:               namespace,
		ingestors:               ingestors,
		batchSigningKeySpec:     &batchSigningKeySpec,
		packetEncryptionKeySpec: &packetEncryptionKeySpec,
	}, nil
}

func (k *Kube) ReconcilePacketEncryptionKey() ([]*PrioKey, error) {
	// #nosec G101
	secretSelector := "type=packet-decryption-key"

	secrets, err := k.getSortedSecretsWithLabel(secretSelector)
	if err != nil {
		return nil, fmt.Errorf("getting the packet encryption keys failed: %w", err)
	}

	// No secret exists, create the first one
	if len(secrets) == 0 {
		key, err := k.createAndStorePacketEncryptionKey()
		if err != nil {
			return nil, fmt.Errorf("creating and storing the packet decyrption keys failed: %w", err)
		}

		return []*PrioKey{key}, nil
	}

	secret := secrets[0]
	keys, err := k.validateAndUpdatePacketEncryptionKey(&secret)
	if err != nil {
		return nil, fmt.Errorf("creating and storing the packet decryption keys failed: %w", err)
	}

	if len(secrets) >= maxSecrets {
		err = k.deleteSecrets(secrets[3:])

		if err != nil {
			return nil, fmt.Errorf("there was an error deleting old secrets. %w", err)
		}
	}

	return keys, nil
}

func (k *Kube) ReconcileBatchSigningKey() (map[string][]*PrioKey, error) {
	results := make(map[string][]*PrioKey)

	for _, ingestor := range k.ingestors {
		secretSelector := fmt.Sprintf("type=batch-signing-key,ingestor=%s", ingestor)
		secretName := fmt.Sprintf(batchSigningKeyFormat, ingestor)

		secrets, err := k.getSortedSecretsWithLabel(secretSelector)
		if err != nil {
			return nil, fmt.Errorf("getting the batch signing secrets failed: %w", err)
		}

		// No secret exists, make the first one.
		if len(secrets) == 0 {
			key, err := k.createAndStoreBatchSigningKey(secretName, ingestor)
			if err != nil {
				return nil, fmt.Errorf("creating and storing the batch signing key failed: %w", err)
			}
			results[ingestor] = []*PrioKey{
				key,
			}
			continue
		}

		// Last good secret
		secret := secrets[0]
		keys, err := k.validateAndUpdateBatchSigningKey(secretName, ingestor, &secret)
		if err != nil {
			return nil, fmt.Errorf("validating the batch signing key failed: %w", err)
		}

		if keys == nil {
			k.log.WithField("ingestor", ingestor).Info("Batch signing key was not expired")
			continue
		}
		results[ingestor] = keys

		if len(secrets) >= maxSecrets {
			err = k.deleteSecrets(secrets[3:])

			if err != nil {
				return nil, fmt.Errorf("there was an error deleting old secrets. %w", err)
			}
		}
	}

	return results, nil
}

func (k *Kube) deleteSecrets(secrets []corev1.Secret) error {
	sApi := k.client.CoreV1().Secrets(k.namespace)

	for _, secret := range secrets {
		err := sApi.Delete(context.Background(), secret.Name, v1.DeleteOptions{GracePeriodSeconds: nil})
		if err != nil {
			return fmt.Errorf("problem with deleting secret %s: %w", secret.Name, err)
		}
	}

	return nil
}

// getSortedSecretWithLabel gets a list of secrets, that were sorted by the secret's creation date
// the label refers to Kubernetes' LabelSelector
func (k *Kube) getSortedSecretsWithLabel(labelSelector string) ([]corev1.Secret, error) {
	sApi := k.client.CoreV1().Secrets(k.namespace)

	secrets, err := sApi.List(context.Background(), v1.ListOptions{
		LabelSelector: labelSelector,
	})

	if err != nil {
		return nil, fmt.Errorf("problem when listing secrets with label %s: %w", labelSelector, err)
	}

	if secrets.Items == nil {
		return nil, fmt.Errorf("secrets was nil after retrieving them from k8s")
	}

	sort.Slice(secrets.Items, func(i, j int) bool {
		item1 := secrets.Items[i]
		item2 := secrets.Items[j]

		time1 := item1.GetCreationTimestamp().Time
		time2 := item2.GetCreationTimestamp().Time

		return time1.After(time2)
	})

	return secrets.Items, nil
}
