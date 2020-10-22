package cert

import (
	"context"
	"fmt"
	"os"
	"regexp"
	"strings"

	"github.com/caddyserver/certmagic"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	v1 "k8s.io/client-go/kubernetes/typed/core/v1"
	"k8s.io/client-go/rest"
)

// KubernetesSecretStorage is a structure that defines the storage system for kubernetes
type KubernetesSecretStorage struct {
	Namespace  string
	KubeClient *kubernetes.Clientset
}

// NewKubernetesSecretStorage creates a new KubernetesSecretStorage for the namespace specified
func NewKubernetesSecretStorage(namespace string) (*KubernetesSecretStorage, error) {
	config, err := rest.InClusterConfig()
	if err != nil {
		return nil, err
	}
	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, err
	}

	return &KubernetesSecretStorage{
		Namespace:  namespace,
		KubeClient: client,
	}, nil
}

var matchLabels = map[string]string{
	"manager": "magiccert",
}
var labelSelector = "manager=magiccert"

var specialChars = regexp.MustCompile("[^a-zA-Z0-9_.-]+")

func cleanKey(key string) string {
	return fmt.Sprintf("acme.%v", specialChars.ReplaceAllString(key, ""))
}

var dataKey = "value"

// Store stores a key and value inside Kubernetes secret storage
func (s *KubernetesSecretStorage) Store(key string, value []byte) error {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:   cleanKey(key),
			Labels: matchLabels,
		},
		Data: map[string][]byte{
			dataKey: value,
		},
	}

	var err error

	secretsAPI := s.getSecretsAPI()
	if s.Exists(key) {
		_, err = secretsAPI.Update(context.Background(), secret, metav1.UpdateOptions{})
	} else {
		_, err = secretsAPI.Create(context.Background(), secret, metav1.CreateOptions{})
	}

	if err != nil {
		return err
	}

	return nil
}

// Load loads a key from kubernetes secret storage
func (s *KubernetesSecretStorage) Load(key string) ([]byte, error) {
	secretsAPI := s.getSecretsAPI()

	secret, err := secretsAPI.Get(context.Background(), cleanKey(key), metav1.GetOptions{})
	if err != nil {
		return nil, err
	}

	data, ok := secret.Data[dataKey]
	if !ok {
		return nil, fmt.Errorf("the data key %s was not included in the secret's data", dataKey)
	}

	return data, nil
}

// Delete deletes a key from kubernetes secret storage
func (s *KubernetesSecretStorage) Delete(key string) error {
	secretsAPI := s.getSecretsAPI()

	return secretsAPI.Delete(context.Background(), cleanKey(key), metav1.DeleteOptions{})
}

// Exists checks if a key exists in kubernetes secret storage
func (s *KubernetesSecretStorage) Exists(key string) bool {
	secrets, err := s.getSecretsAPI().List(context.Background(), metav1.ListOptions{
		FieldSelector: fmt.Sprintf("metadata.name=%v", cleanKey(key)),
	})

	if err != nil {
		_, _ = fmt.Fprintln(os.Stderr, err)
		return false
	}

	for _, item := range secrets.Items {
		if item.ObjectMeta.Name == cleanKey(key) {
			return true
		}
	}

	return false
}

// List lists a keys starting with any given prefix in kubernetes secret storage
func (s *KubernetesSecretStorage) List(prefix string, _ bool) ([]string, error) {
	secretsAPI := s.getSecretsAPI()

	secrets, err := secretsAPI.List(context.Background(), metav1.ListOptions{
		LabelSelector: labelSelector,
	})

	if err != nil {
		return nil, err
	}

	var keys []string
	for _, secret := range secrets.Items {
		key := secret.ObjectMeta.Name
		if strings.HasPrefix(key, cleanKey(prefix)) {
			keys = append(keys, key)
		}
	}
	return keys, nil
}

// Stat returns the information about a key from kubernetes secret storage
func (s *KubernetesSecretStorage) Stat(key string) (certmagic.KeyInfo, error) {
	secretsAPI := s.getSecretsAPI()

	secret, err := secretsAPI.Get(context.Background(), cleanKey(key), metav1.GetOptions{})
	if err != nil {
		return certmagic.KeyInfo{}, err
	}

	return certmagic.KeyInfo{
		Key:        key,
		Modified:   secret.GetCreationTimestamp().UTC(),
		Size:       int64(len(secret.Data[dataKey])),
		IsTerminal: true,
	}, nil
}

// Lock is not implemented
func (s *KubernetesSecretStorage) Lock(ctx context.Context, key string) error {
	// Do we need this? If so how should we implement this?
	return nil
}

// Unlock is not implemented
func (s *KubernetesSecretStorage) Unlock(key string) error {
	// See KubernetesSecretStorage#Lock
	return nil
}

func (s *KubernetesSecretStorage) getSecretsAPI() v1.SecretInterface {
	return s.KubeClient.CoreV1().Secrets(s.Namespace)
}
