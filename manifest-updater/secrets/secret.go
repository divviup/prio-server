package secrets

import (
	"context"
	"github.com/pkg/errors"
	log "github.com/sirupsen/logrus"
	kubeError "k8s.io/apimachinery/pkg/api/errors"
	v1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
	"os"
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
	batchSigningKeyFormat   = "batch-signing-key-%s"
)

func NewKube(namespace string, dataShareProcessors []string) (*Kube, error) {
	config, err := rest.InClusterConfig()
	if err != nil {
		config, err = clientcmd.BuildConfigFromFlags("", os.Getenv("KUBECONFIG"))
		if err != nil {
			return nil, errors.Wrap(err, "failed to get the KUBECONFIG")
		}
	}

	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, errors.Wrap(err, "getting the client failed")
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
				return nil, errors.Wrap(err, "creating and storing the packet decryption key failed")
			}

			return key, nil
		}
		return nil, errors.Wrap(err, "problem getting the secrets")
	}

	key, err := k.validateAndUpdatePacketEncryptionKey(secret)
	if err != nil {
		return nil, errors.Wrap(err, "creating and storing the packet decryption key failed")
	}

	return key, nil
}

//func (k *Kube) ReconcileBatchSigningKey() (map[string]*KubernetesBatchSigningKey, error) {
//	sApi := k.client.CoreV1().Secrets(k.namespace)
//	results := make(map[string]*KubernetesBatchSigningKey)
//
//	for _, dataShareProcessor := range k.dataShareProcessors {
//
//		keyName := fmt.Sprintf(batchSigningKeyFormat, dataShareProcessor)
//
//		keys, err := sApi.List(context.Background(), v1.ListOptions{
//			LabelSelector: fmt.Sprintf("type=batch-signing-key,dsp=%s", dataShareProcessor),
//		})
//
//		if err != nil {
//			return nil, fmt.Errorf("problem when listing the batch-signing-keys: %w", err)
//		}
//
//		// Sorts so the items are newest first
//		sort.Slice(keys.Items, func(i, j int) bool {
//			item1 := keys.Items[i]
//			item2 := keys.Items[j]
//
//			time1 := item1.GetCreationTimestamp().Time
//			time2 := item2.GetCreationTimestamp().Time
//
//			return time1.After(time2)
//		})
//
//		if len(keys.Items) == 0 {
//			cert, err := k.createAndStoreBatchSigningKey(keyName, dataShareProcessor)
//			if err != nil {
//				return nil, fmt.Errorf("creating and storing the batch signing key failed: %w", err)
//			}
//
//			results[dataShareProcessor] = cert
//			continue
//		}
//
//		secret := keys.Items[0]
//		batchSigningKeyManifest, err := k.validateAndUpdateBatchSigningKey(keyName, dataShareProcessor, &secret)
//		if err != nil {
//			return nil, fmt.Errorf("validating the batch signing key failed: %w", err)
//		}
//
//		results[dataShareProcessor] = batchSigningKeyManifest
//	}
//
//	return results, nil
//}
