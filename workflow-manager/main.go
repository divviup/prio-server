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
	"io/ioutil"
	"log"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/credentials"
	"github.com/aws/aws-sdk-go/aws/credentials/stscreds"
	aws_session "github.com/aws/aws-sdk-go/aws/session"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/aws/aws-sdk-go/service/sts"

	"cloud.google.com/go/storage"
	"google.golang.org/api/iterator"
	batchv1 "k8s.io/api/batch/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/rest"
)

// BuildID is generated at build time and contains the branch and short hash.
var BuildID string

//BuildTime is generated at build time and contains the build time.
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
var maxAge = flag.String("max-age", "1h", "Max age (in Go duration format) for batches to be worth processing.")
var k8sServiceAccount = flag.String("k8s-service-account", "", "Kubernetes service account for intake and aggregate jobs")
var bskSecretName = flag.String("bsk-secret-name", "", "Name of k8s secret for batch signing key")
var pdksSecretName = flag.String("pdks-secret-name", "", "Name of k8s secret for packet decrypt keys")
var intakeConfigMap = flag.String("intake-batch-config-map", "", "Name of config map for intake jobs")
var aggregateConfigMap = flag.String("aggregate-config-map", "", "Name of config map for aggregate jobs")

func main() {
	log.Printf("starting %s version %s - %s. Args: %s", os.Args[0], BuildID, BuildTime, os.Args[1:])

	inputBucket := flag.String("input-bucket", "", "Name of input bucket (required)")
	service := flag.String("service", "s3", "Where to find buckets (s3 or gs)")
	flag.Parse()
	if *inputBucket == "" {
		flag.Usage()
		os.Exit(1)
	}

	if *intakeConfigMap == "" || *aggregateConfigMap == "" {
		log.Fatal("--intake-batch-config-map and --aggregate-config-map are required")
	}

	ageLimit, err := time.ParseDuration(*maxAge)
	if err != nil {
		log.Fatal(err)
	}

	var readyBatches []string
	switch *service {
	case "s3":
		readyBatches, err = getReadyBatchesS3(context.Background(), *inputBucket, os.Getenv("AWS_ROLE_ARN"))
		if err != nil {
			log.Fatal(err)
		}
	case "gs":
		readyBatches, err = getReadyBatchesGS(context.Background(), *inputBucket)
		if err != nil {
			log.Fatal(err)
		}
	default:
		log.Fatalf("unknown service %s", *service)
	}

	if err := launch(context.Background(), readyBatches, ageLimit); err != nil {
		log.Fatal(err)
	}
	log.Print("done")
}

type tokenFetcher struct {
	audience string
}

func (tf tokenFetcher) FetchToken(credentials.Context) ([]byte, error) {
	url := fmt.Sprintf("http://metadata.google.internal:80/computeMetadata/v1/instance/service-accounts/default/identity?audience=%s", tf.audience)
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return nil, fmt.Errorf("building request: %w", err)
	}
	req.Header.Add("Metadata-Flavor", "Google")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("fetching %s: %w", url, err)
	}
	bytes, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("reading body of %s: %w", url, err)
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("status code %d from metadata service at %s: %s",
			resp.StatusCode, url, string(bytes))
	}
	log.Printf("fetched token from %s", url)
	return bytes, nil
}

func getReadyBatchesS3(ctx context.Context, inputBucket string, roleARN string) ([]string, error) {
	parts := strings.SplitN(inputBucket, "/", 2)
	region := parts[0]
	bucket := parts[1]
	sess, err := aws_session.NewSession()
	if err != nil {
		return nil, fmt.Errorf("making AWS session: %w", err)
	}

	arnComponents := strings.Split(roleARN, ":")
	if len(arnComponents) != 6 {
		return nil, fmt.Errorf("invalid ARN: %q", roleARN)
	}
	audience := fmt.Sprintf("sts.amazonaws.com/%s", arnComponents[4])

	stsSTS := sts.New(sess)
	roleSessionName := ""
	roleProvider := stscreds.NewWebIdentityRoleProviderWithToken(
		stsSTS, roleARN, roleSessionName, tokenFetcher{audience})

	credentials := credentials.NewCredentials(roleProvider)

	config := aws.NewConfig().
		WithRegion(region).
		WithCredentials(credentials)
	svc := s3.New(sess, config)
	resp, err := svc.ListObjectsV2(&s3.ListObjectsV2Input{Bucket: aws.String(bucket)})
	if err != nil {
		return nil, fmt.Errorf("Unable to list items in bucket %q, %w", inputBucket, err)
	}
	batches := make(map[string]*batch)
	for _, item := range resp.Contents {
		name := *item.Key
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
	return readyBatches, nil
}

func getReadyBatchesGS(ctx context.Context, inputBucket string) ([]string, error) {
	client, err := storage.NewClient(ctx)
	if err != nil {
		return nil, fmt.Errorf("storage.newClient: %w", err)
	}

	batches := make(map[string]*batch)
	bkt := client.Bucket(inputBucket)
	query := &storage.Query{Prefix: ""}
	it := bkt.Objects(ctx, query)
	for {
		attrs, err := it.Next()
		if err == iterator.Done {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("iterating on inputBucket objects from %q: %w", inputBucket, err)
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

	return readyBatches, nil
}

func launch(ctx context.Context, readyBatches []string, ageLimit time.Duration) error {
	// This uses the credentials that an instance running in the k8s cluster
	// gets automatically, via automount_service_account_token in the Terraform config.
	config, err := rest.InClusterConfig()
	if err != nil {
		return fmt.Errorf("cluster config: %w", err)
	}
	clientset, err := kubernetes.NewForConfig(config)
	if err != nil {
		return fmt.Errorf("clientset: %w", err)
	}

	log.Printf("starting %d jobs", len(readyBatches))
	for _, batchName := range readyBatches {
		if err := startJob(ctx, clientset, batchName, ageLimit); err != nil {
			return fmt.Errorf("starting job for batch %q: %w", batchName, err)
		}
	}
	return nil
}

func startJob(
	ctx context.Context,
	clientset *kubernetes.Clientset,
	batchName string,
	ageLimit time.Duration,
) error {
	// batchName is like "kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771"
	pathComponents := strings.Split(batchName, "/")
	batchID := pathComponents[len(pathComponents)-1]
	aggregationID := pathComponents[0]
	batchDate := pathComponents[1 : len(pathComponents)-1]

	if len(batchDate) != 5 {
		return fmt.Errorf("malformed date in %q. Expected 5 date components, got %d", batchName, len(batchDate))
	}
	var dateComponents []int
	for _, c := range batchDate {
		parsed, err := strconv.ParseInt(c, 10, 64)
		if err != nil {
			return fmt.Errorf("parsing date component %q in %q: %w", c, batchName, err)
		}
		dateComponents = append(dateComponents, int(parsed))
	}

	batchTime := time.Date(dateComponents[0], time.Month(dateComponents[1]), dateComponents[2], dateComponents[3], dateComponents[4], 0, 0, nil)
	age := time.Now().Sub(batchTime)
	if age > ageLimit {
		log.Printf("skipping batch %q because it is too old (%s)", batchName, age)
	}

	jobName := fmt.Sprintf("i-batch-%s", batchID)

	args := []string{
		"intake-batch",
		"--aggregation-id", aggregationID,
		"--batch-id", batchID,
		"--date", strings.Join(batchDate, "/"),
	}
	log.Printf("starting job for batch %q with args %s", batchName, args)
	job := &batchv1.Job{
		ObjectMeta: metav1.ObjectMeta{
			Name:      jobName,
			Namespace: *k8sNS,
		},
		Spec: batchv1.JobSpec{
			Template: corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{
					ServiceAccountName: *k8sServiceAccount,
					RestartPolicy:      "Never",
					Containers: []corev1.Container{
						{
							Args:            args,
							Name:            "facile-container",
							Image:           "us.gcr.io/jsha-prio-bringup/letsencrypt/prio-facilitator:1.2.3",
							ImagePullPolicy: "Always",
							EnvFrom: []corev1.EnvFromSource{
								{
									ConfigMapRef: &corev1.ConfigMapEnvSource{
										LocalObjectReference: corev1.LocalObjectReference{
											Name: *intakeConfigMap,
										},
									},
								},
							},
							Env: []corev1.EnvVar{
								{
									Name: "BATCH_SIGNING_KEY",
									ValueFrom: &corev1.EnvVarSource{
										SecretKeyRef: &corev1.SecretKeySelector{
											LocalObjectReference: corev1.LocalObjectReference{
												Name: *bskSecretName,
											},
											Key: "secret_key",
										},
									},
								},
								{
									Name: "PACKET_DECRYPTION_KEYS",
									ValueFrom: &corev1.EnvVarSource{
										SecretKeyRef: &corev1.SecretKeySelector{
											LocalObjectReference: corev1.LocalObjectReference{
												Name: *pdksSecretName,
											},
											Key: "secret_key",
										},
									},
								},
							},
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
		}
		return fmt.Errorf("creating job: %w", err)
	}
	log.Printf("Created job %q: %#v", jobName, createdJob.ObjectMeta.UID)

	return nil
}
