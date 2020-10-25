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

type batch struct {
	metadata bool
	avro     bool
	sig      bool
}

func basename(s string) string {
	s = strings.TrimSuffix(s, ".batch")
	s = strings.TrimSuffix(s, ".batch.avro")
	s = strings.TrimSuffix(s, ".batch.sig")
	return s
}

func main() {
	inputBucket := flag.String("input-bucket", "", "Name of input bucket (required)")
	flag.Parse()
	if *inputBucket == "" {
		flag.Usage()
	}

	log.Printf("starting %s", os.Args[0])
	if err := work(*inputBucket); err != nil {
		log.Fatal(err)
	}
	log.Print("done")
}

func work(inputBucket string) error {
	config, err := rest.InClusterConfig()
	if err != nil {
		return fmt.Errorf("cluster config: %w", err)
	}
	// creates the clientset
	clientset, err := kubernetes.NewForConfig(config)
	if err != nil {
		return fmt.Errorf("clientset: %w", err)
	}

	pods, err := clientset.CoreV1().Pods("").List(context.TODO(), metav1.ListOptions{})
	if err != nil {
		return fmt.Errorf("getting pods: %w", err)
	}
	log.Printf("There are %d pods in the cluster", len(pods.Items))

	ctx := context.Background()
	client, err := storage.NewClient(ctx)
	if err != nil {
		return fmt.Errorf("storage.newClient: %w", err)
	}

	batches := make(map[string]*batch)
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
