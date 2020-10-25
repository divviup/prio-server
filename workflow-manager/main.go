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

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/rest"

	"cloud.google.com/go/storage"
	"google.golang.org/api/iterator"
)

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

func main() {
	inputBucket := flag.String("input-bucket", "", "Name of input bucket (required)")
	k8sNS := flag.String("k8s-namespace", "", "Kubernetes namespace")
	flag.Parse()
	if *inputBucket == "" {
		flag.Usage()
	}

	log.Printf("starting %s", os.Args[0])
	if err := work(*inputBucket, *k8sNS); err != nil {
		log.Fatal(err)
	}
	log.Print("done")
}

func work(inputBucket, k8sNS string) error {
	config, err := rest.InClusterConfig()
	if err != nil {
		return fmt.Errorf("cluster config: %w", err)
	}
	// creates the clientset
	clientset, err := kubernetes.NewForConfig(config)
	if err != nil {
		return fmt.Errorf("clientset: %w", err)
	}

	pods, err := clientset.CoreV1().Pods(k8sNS).List(context.TODO(), metav1.ListOptions{})
	if err != nil {
		return fmt.Errorf("getting pods: %w", err)
	}
	log.Printf("There are %d pods in the cluster", len(pods.Items))

	ctx := context.Background()
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
			return fmt.Errorf("iterating on inputBucket objects: %w", err)
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

	var readybatches []*batch
	var unreadybatches []*batch
	for k, v := range batches {
		if v.metadata && v.avro && v.sig {
			log.Printf("ready: %s", k)
			readybatches = append(readybatches, v)
		} else {
			log.Printf("unready: %s", k)
			unreadybatches = append(unreadybatches, v)
		}
	}
	return nil
}
