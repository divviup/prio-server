package secrets

import (
	"context"
	"fmt"
	log "github.com/sirupsen/logrus"
	kubeError "k8s.io/apimachinery/pkg/api/errors"
	v1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
	"os"
	"sort"
)

type Kube struct {
	log                 *log.Entry
	client              *kubernetes.Clientset
	namespace           string
	dataShareProcessors []string
}

const (
	packetDecryptionKeyName = "packet-decryption-key"
	secretKeyMap            = "secret_key"
	batchSigningKeyFormat   = "batch-signing-key-%s-"
)

func NewKube(namespace string, dataShareProcessors []string) (*Kube, error) {
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
		log:                 log.WithField("source", "kube secret").WithField("namespace", namespace),
		client:              client,
		namespace:           namespace,
		dataShareProcessors: dataShareProcessors,
	}, nil
}

func (k *Kube) ReconcilePacketEncryptionKey() (*PrioKey, error) {
	sApi := k.client.CoreV1().Secrets(k.namespace)

	secret, err := sApi.Get(context.Background(), packetDecryptionKeyName, v1.GetOptions{})

	if err != nil {
		if kubeError.IsNotFound(err) {
			key, err := k.createAndStorePacketEncryptionKey()
			if err != nil {
				return nil, fmt.Errorf("creating and storing the packet decryption key failed: %w", err)
			}

			return key, nil
		}
		return nil, fmt.Errorf("problem getting the secrets: %w", err)
	}

	key, err := k.validateAndUpdatePacketEncryptionKey(secret)
	if err != nil {
		return nil, fmt.Errorf("creating and storing the packet decryption key failed: %w", err)
	}

	return key, nil
}

func (k *Kube) ReconcileBatchSigningKey() (map[string][]*PrioKey, error) {
	sApi := k.client.CoreV1().Secrets(k.namespace)
	results := make(map[string][]*PrioKey)

	for _, dataShareProcessor := range k.dataShareProcessors {

		secretName := fmt.Sprintf(batchSigningKeyFormat, dataShareProcessor)

		secretList, err := sApi.List(context.Background(), v1.ListOptions{
			LabelSelector: fmt.Sprintf("type=batch-signing-key,dsp=%s", dataShareProcessor),
		})

		if err != nil {
			return nil, fmt.Errorf("problem when listing the batch-signing-secretList: %w", err)
		}

		// Sorts so the items are newest first
		sort.Slice(secretList.Items, func(i, j int) bool {
			item1 := secretList.Items[i]
			item2 := secretList.Items[j]

			time1 := item1.GetCreationTimestamp().Time
			time2 := item2.GetCreationTimestamp().Time

			return time1.After(time2)
		})

		if len(secretList.Items) == 0 {
			key, err := k.createAndStoreBatchSigningKey(secretName, dataShareProcessor)
			if err != nil {
				return nil, fmt.Errorf("creating and storing the batch signing key failed: %w", err)
			}
			results[dataShareProcessor] = []*PrioKey{
				key,
			}
			continue
		}

		secret := secretList.Items[0]
		keys, err := k.validateAndUpdateBatchSigningKey(secretName, dataShareProcessor, &secret)
		if err != nil {
			return nil, fmt.Errorf("validating the batch signing key failed: %w", err)
		}

		results[dataShareProcessor] = keys
	}

	return results, nil
}
