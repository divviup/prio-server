// package kubernetes contains utilities related to Kubernetes Jobs
package kubernetes

import (
	"context"
	"fmt"
	"sort"

	"github.com/letsencrypt/prio-server/workflow-manager/utils"

	batchv1 "k8s.io/api/batch/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/tools/clientcmd"
)

type Client struct {
	client    *kubernetes.Clientset
	namespace string
	dryRun    bool
}

// Client returns a Clientset that uses the credentials that an instance running
// in the k8s cluster gets automatically, via automount_service_account_token in
// the Terraform config, or the credentials in the provided kube config file, if
// it is not empty. If dryRun is true, then any destructive API calls will be
// made with DryRun: All.
func NewClient(namespace string, kubeconfigPath string, dryRun bool) (*Client, error) {
	// BuildConfigFromFlags falls back to rest.InClusterConfig if kubeconfigPath
	// is empty
	// https://godoc.org/k8s.io/client-go/tools/clientcmd#BuildConfigFromFlags
	config, err := clientcmd.BuildConfigFromFlags("", kubeconfigPath)
	if err != nil {
		return nil, fmt.Errorf("cluster config: %w", err)
	}

	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, fmt.Errorf("clientset: %w", err)
	}

	return &Client{
		client:    client,
		namespace: namespace,
		dryRun:    dryRun,
	}, nil
}

// ListAllJobs returns a map of Kubernetes jobs in the specified namespace, where
// the key is the name of the job and the value is the job structure, or an
// error on failure.
func (c *Client) ListAllJobs() (map[string]batchv1.Job, error) {
	return c.ListJobs(metav1.ListOptions{})
}

// ListJobs returns a map of Kubernetes jobs in the specified namespace.
// the options allows filtering the list results, the Limit and Continue fields of the
// ListOptions will be overwritten. The key of the map is the name of the job, the value
// is the job structure.
func (c *Client) ListJobs(options metav1.ListOptions) (map[string]batchv1.Job, error) {
	jobs := map[string]batchv1.Job{}

	// The jobs list API is paginated. We request 1000 entries at a time. If
	// there are more entries, the response will contain a continue token to
	// provide on subsequent requests.
	continueToken := ""
	for {
		ctx, cancel := utils.ContextWithTimeout()
		defer cancel()

		options.Limit = 1000
		options.Continue = continueToken
		jobsList, err := c.client.BatchV1().Jobs(c.namespace).List(ctx, options)
		if err != nil {
			return nil, fmt.Errorf("failed to list jobs in namespace: %w", err)
		}

		for _, job := range jobsList.Items {
			jobs[job.Name] = job
		}

		if jobsList.Continue == "" {
			break
		}

		continueToken = jobsList.Continue
	}

	return jobs, nil
}

// ScheduleJob schedules job at a given namespace and returns the created job
func (c *Client) ScheduleJob(job *batchv1.Job) (*batchv1.Job, error) {
	createdJob, err := c.client.BatchV1().Jobs(c.namespace).Create(context.Background(), job, metav1.CreateOptions{})
	if err != nil {
		return nil, fmt.Errorf("job creation failed: %v", err)
	}

	return createdJob, nil
}

// RemoveJobCollection removes a collection of jobs defined by the listOptions.
// The DryRun field of DeleteOptions will be overwritten based on how the client is configured
func (c *Client) RemoveJobCollection(deleteOptions metav1.DeleteOptions, listOptions metav1.ListOptions) error {
	if c.dryRun {
		deleteOptions.DryRun = []string{"All"}
	}
	err := c.client.BatchV1().Jobs(c.namespace).DeleteCollection(context.Background(), deleteOptions, listOptions)

	if err != nil {
		return fmt.Errorf("deleting job collection failed: %v", err)
	}

	return nil
}

// GetSortedSecrets gets a list of secrets that were sorted by the secret's creation date (newest secret first)
func (c *Client) GetSortedSecrets(labelSelector string) ([]corev1.Secret, error) {
	secrets, err := c.client.CoreV1().Secrets(c.namespace).List(context.Background(), metav1.ListOptions{LabelSelector: labelSelector})

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
