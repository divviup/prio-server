// package kubernetes contains utilities related to Kubernetes Jobs
package kubernetes

import (
	"fmt"
	"net"
	"os"
	"time"

	"gopkg.in/retry.v1"

	"github.com/letsencrypt/prio-server/workflow-manager/utils"

	batchv1 "k8s.io/api/batch/v1"
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
	// Making sure endpoint is available before returning kubernetes client
	// only for inCluster config
	host, port := os.Getenv("KUBERNETES_SERVICE_HOST"), os.Getenv("KUBERNETES_SERVICE_PORT")
	if kubeconfigPath == "" && host != "" && port != "" {
		available := false
		strategy := retry.LimitTime(60*time.Second,
			retry.Exponential{
				Initial: 500 * time.Millisecond,
				Factor:  1.2,
			},
		)
		timeout := time.Duration(1 * time.Second)
		for a := retry.Start(strategy, nil); a.Next(); {
			if _, err := net.DialTimeout("tcp", host+":"+port, timeout); err == nil {
				available = true
				break
			}
		}
		if !available {
			return nil, fmt.Errorf("InCluster k8s api endpoint not available")
		}
	}

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

// ListJobs returns a map of Kubernetes jobs in the specified namespace, where
// the key is the name of the job and the value is the job structure, or an
// error on failure.
func (c *Client) ListJobs() (map[string]batchv1.Job, error) {
	jobs := map[string]batchv1.Job{}

	// The jobs list API is paginated. We request 1000 entries at a time. If
	// there are more entries, the response will contain a continue token to
	// provide on subsequent requests.
	continueToken := ""
	for {
		ctx, cancel := utils.ContextWithTimeout()
		defer cancel()
		jobsList, err := c.client.BatchV1().Jobs(c.namespace).List(ctx, metav1.ListOptions{
			Limit:    1000,
			Continue: continueToken,
		})
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
