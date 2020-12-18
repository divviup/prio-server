package kubernetes

import (
	"context"
	"fmt"
	"os"
	"sort"

	batchv1 "k8s.io/api/batch/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
)

// GetSortedSecrets gets a list of secrets that were sorted by the secret's creation date
func GetSortedSecrets(namespace, labelSelector string) ([]corev1.Secret, error) {
	client, err := getKubernetes()
	if err != nil {
		return nil, err
	}

	secrets, err := client.CoreV1().Secrets(namespace).List(context.Background(), metav1.ListOptions{LabelSelector: labelSelector})

	if err != nil {
		return nil, fmt.Errorf("problem when listing secrets with label %s: %v", labelSelector, err)
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

func ScheduleJob(namespace string, job *batchv1.Job) error {
	client, err := getKubernetes()
	if err != nil {
		return err
	}

	_, err = client.BatchV1().Jobs(namespace).Create(context.Background(), job, metav1.CreateOptions{})
	if err != nil {
		return fmt.Errorf("job creation failed: %v", err)
	}

	return nil
}

func getKubernetes() (*kubernetes.Clientset, error) {
	config, err := rest.InClusterConfig()
	if err != nil {
		config, err = clientcmd.BuildConfigFromFlags("", os.Getenv("KUBECONFIG"))
		if err != nil {
			return nil, fmt.Errorf("failed to get KUBECONFIG environment variable: %v", err)
		}
	}

	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, fmt.Errorf("client not created: %v", err)
	}

	return client, nil
}
