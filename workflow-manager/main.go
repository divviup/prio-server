// workflow-manager looks for batches to be processed from an input bucket,
// and spins up facilitator jobs to process those batches.
//
// Right now workflow-manager is just a stub that demonstrates reading from a
// GCS bucket and talking to the Kubernetes API.
package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"os"
	"strings"

	batchv1 "k8s.io/api/batch/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/rest"

	"cloud.google.com/go/storage"
	"google.golang.org/api/iterator"
)

var BuildID string
var BuildTime string

// batchMap is used to store information about a batch, indexed by the basename
// of that batch.
type batchMap map[string]*batch

// batch is used to determine whether all elements of a batch are present.
type batch struct {
	metadata bool
	avro     bool
	sig      bool
}

// basename returns the full name of the batch with any of the type suffixes
// removed. All files that are part of a batch have the same basename.
func basename(s string) string {
	s = strings.TrimSuffix(s, ".batch")
	s = strings.TrimSuffix(s, ".batch.avro")
	s = strings.TrimSuffix(s, ".batch.sig")
	return s
}

var k8sNS = flag.String("k8s-namespace", "", "Kubernetes namespace")

func main() {
	inputBucket := flag.String("input-bucket", "", "Name of input bucket (required)")
	flag.Parse()
	if *inputBucket == "" {
		flag.Usage()
	}

	log.Printf("starting %s", os.Args[0])
	if err := work(context.Background(), *inputBucket); err != nil {
		log.Fatal(err)
	}
	log.Print("done")
}

func work(ctx context.Context, inputBucket string) error {
	config, err := rest.InClusterConfig()
	if err != nil {
		return fmt.Errorf("cluster config: %w", err)
	}
	// creates the clientset
	clientset, err := kubernetes.NewForConfig(config)
	if err != nil {
		return fmt.Errorf("clientset: %w", err)
	}

	client, err := storage.NewClient(ctx)
	if err != nil {
		return fmt.Errorf("storage.newClient: %w", err)
	}

	batches := make(batchMap)
	bkt := client.Bucket(inputBucket)
	query := &storage.Query{Prefix: ""}
	var names []string
	it := bkt.Objects(ctx, query)
	for {
		attrs, err := it.Next()
		if err == iterator.Done {
			break
		}
		if err != nil {
			return fmt.Errorf("iterating on inputBucket objects from %q: %w", inputBucket, err)
		}
		name := attrs.Name
		basename := basename(name)
		if batches[basename] == nil {
			batches[basename] = new(batch)
		}
		b := batches[basename]
		if strings.HasSuffix(name, ".batch") {
			b.metadata = true
		}
		if strings.HasSuffix(name, ".batch.avro") {
			b.avro = true
		}
		if strings.HasSuffix(name, ".batch.sig") {
			b.sig = true
		}
		names = append(names, attrs.Name)
	}

	var readyBatches []string
	for k, v := range batches {
		if v.metadata && v.avro && v.sig {
			log.Printf("ready: %s", k)
			readyBatches = append(readyBatches, k)
		} else {
			log.Printf("unready: %s", k)
		}
	}

	log.Printf("starting %d jobs", len(readyBatches))
	for _, batchName := range readyBatches {
		err = startJob(ctx, clientset, batchName)
		if err != nil {
			return fmt.Errorf("starting job for batch %q: %w", batchName, err)
		}
	}
	return nil
}

func startJob(ctx context.Context, clientset *kubernetes.Clientset, batchName string) error {
	log.Printf("starting job for batch %q...", batchName)
	jobName := fmt.Sprintf("i-batch-%s", strings.ReplaceAll(batchName, "/", "-"))

	pathComponents := strings.Split(batchName, "/")
	batchID := pathComponents[len(pathComponents)-1]
	batchDate := strings.Join(pathComponents[0:len(pathComponents)-1], "/")
	job := &batchv1.Job{
		ObjectMeta: metav1.ObjectMeta{
			Name:      jobName,
			Namespace: *k8sNS,
		},
		Spec: batchv1.JobSpec{
			Template: corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{
					RestartPolicy: "Never",
					Containers: []corev1.Container{
						{
							Args: []string{
								"batch-intake",
								"--s3-use-credentials-from-gke-metadata",
								"--aggregation-id", "fake-1",
								"--batch-id", batchID,
								"--date", batchDate,
								"--ingestion-bucket", "gs://tbd",
								"--validation-bucket", "gs://tbd",
								"--ingestor-public-key", "what_is_my_pubkey",
								"--ecies-private-key", "what_is_my_privkey",
								"--share-processor-private-key", "what_is_my_other_privkey",
							},
							Name:            "facile-container",
							Image:           "us.gcr.io/jsha-prio-bringup/letsencrypt/prio-facilitator:7.7.7",
							ImagePullPolicy: "Always",
						},
					},
				},
			},
		},
	}
	createdJob, err := clientset.BatchV1().Jobs(*k8sNS).Create(ctx, job, metav1.CreateOptions{})
	if err != nil {
		// TODO: Checking an error for a substring is pretty clumsy. Figure out if k8s client
		// returns typed errors.
		if strings.HasSuffix(err.Error(), "already exists") {
			log.Printf("skipping %q because a job for it already exists (err %T = %#v)",
				batchName, err, err)
			return nil
		} else {
			return fmt.Errorf("creating job: %w", err)
		}
	}
	log.Printf("Created job %q: %#v", jobName, createdJob.ObjectMeta.UID)

	return nil
}
