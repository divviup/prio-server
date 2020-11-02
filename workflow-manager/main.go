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
	"k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/resource"
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

type batchPath struct {
	aggregationID  string
	dateComponents []string
	ID             string
	time           time.Time
}

func newBatchPath(batchName string) (batchPath, error) {
	// batchName is like "kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771"
	pathComponents := strings.Split(batchName, "/")
	batchID := pathComponents[len(pathComponents)-1]
	aggregationID := pathComponents[0]
	batchDate := pathComponents[1 : len(pathComponents)-1]

	if len(batchDate) != 5 {
		return batchPath{}, fmt.Errorf("malformed date in %q. Expected 5 date components, got %d", batchName, len(batchDate))
	}

	var dateComponents []int
	for _, c := range batchDate {
		parsed, err := strconv.ParseInt(c, 10, 64)
		if err != nil {
			return batchPath{}, fmt.Errorf("parsing date component %q in %q: %w", c, batchName, err)
		}
		dateComponents = append(dateComponents, int(parsed))
	}
	batchTime := time.Date(dateComponents[0], time.Month(dateComponents[1]),
		dateComponents[2], dateComponents[3], dateComponents[4], 0, 0, time.UTC)

	return batchPath{
		aggregationID:  aggregationID,
		dateComponents: batchDate,
		ID:             batchID,
		time:           batchTime,
	}, nil
}

func (b *batchPath) path() string {
	return strings.Join([]string{b.aggregationID, b.dateString(), b.ID}, "/")
}

func (b *batchPath) dateString() string {
	return strings.Join(b.dateComponents, "/")
}

// ingestionBasename returns the full name of the batch with any of the type
// suffixes removed. All files that are part of a batch have the same basename.
func ingestionBasename(s string) string {
	s = strings.TrimSuffix(s, ".batch")
	s = strings.TrimSuffix(s, ".batch.avro")
	s = strings.TrimSuffix(s, ".batch.sig")
	return s
}

// validationBasename returns the full name of the batch with any of the type
// suffixes removed. All files that are part of a batch have the same basename.
func validationBasename(s string, isFirst bool) string {
	s = strings.TrimSuffix(s, fmt.Sprintf(".validity_%s", index(isFirst)))
	s = strings.TrimSuffix(s, fmt.Sprintf(".validity_%s.avro", index(isFirst)))
	s = strings.TrimSuffix(s, fmt.Sprintf(".validity_%s.sig", index(isFirst)))
	return s
}

// index returns the appropriate string to use in construction of filenames
// based on whether the file creator was the "first" aka PHA server.
func index(isFirst bool) string {
	if isFirst {
		return "0"
	}
	return "1"
}

// contains returns true if the string str exists in the slice s
func contains(s []string, str string) bool {
	for _, v := range s {
		if v == str {
			return true
		}
	}

	return false
}

// checkStoragePath checks that the provided value is present and that it is
// either an S3 or GCS bucket URL, and aborts the program with an error message
// if not. Otherwise returns a tuple whose first member is the storage service
// as "s3" or "gs", and whose second is the bucket name (including region, in
// the S3 case).
func checkStoragePath(arg string, val string) (string, string) {
	if val == "" {
		log.Fatalf("%s is required", arg)
	}
	if !strings.HasPrefix(val, "s3://") && !strings.HasPrefix(val, "gs://") {
		log.Fatalf("invalid %s: %q", arg, val)
	}

	return val[0:2], val[5:]
}

var k8sNS = flag.String("k8s-namespace", "", "Kubernetes namespace")
var isFirst = flag.Bool("is-first", false, "Whether this set of servers is \"first\", aka PHA servers")
var maxAge = flag.String("max-age", "1h", "Max age (in Go duration format) for batches to be worth processing.")
var k8sServiceAccount = flag.String("k8s-service-account", "", "Kubernetes service account for intake and aggregate jobs")
var bskSecretName = flag.String("bsk-secret-name", "", "Name of k8s secret for batch signing key")
var pdksSecretName = flag.String("pdks-secret-name", "", "Name of k8s secret for packet decrypt keys")
var intakeConfigMap = flag.String("intake-batch-config-map", "", "Name of config map for intake jobs")
var aggregateConfigMap = flag.String("aggregate-config-map", "", "Name of config map for aggregate jobs")
var ingestorInput = flag.String("ingestor-input", "", "Bucket for input from ingestor (s3:// or gs://) (Required)")
var ingestorIdentity = flag.String("ingestor-identity", "", "Identity to use with ingestor bucket (Required for S3)")
var ownValidationInput = flag.String("own-validation-input", "", "Bucket for input of validation batches from self (s3:// or gs://) (required)")
var ownValidationIdentity = flag.String("own-validation-identity", "", "Identity to use with own validation bucket (Required for S3)")
var peerValidationInput = flag.String("peer-validation-input", "", "Bucket for input of validation batches from peer (s3:// or gs://) (required)")
var peerValidationIdentity = flag.String("peer-validation-identity", "", "Identity to use with peer validation bucket (Required for S3)")
var facilitatorImage = flag.String("facilitator-image", "", "Name (optionally including repository) of facilitator image")

func main() {
	log.Printf("starting %s version %s - %s. Args: %s", os.Args[0], BuildID, BuildTime, os.Args[1:])
	flag.Parse()

	ingestorService, ingestorInputBucket := checkStoragePath("--ingestor-input", *ingestorInput)
	ownValidationService, ownValidationInputBucket := checkStoragePath("--own-validation-input", *ownValidationInput)
	peerValidationService, peerValidationInputBucket := checkStoragePath("--peer-validation-input", *peerValidationInput)

	if *intakeConfigMap == "" || *aggregateConfigMap == "" {
		log.Fatal("--intake-batch-config-map and --aggregate-config-map are required")
	}
	if *facilitatorImage == "" {
		log.Fatal("--facilitator-image is required")
	}

	ageLimit, err := time.ParseDuration(*maxAge)
	if err != nil {
		log.Fatal(err)
	}

	var intakeBatches []string
	switch ingestorService {
	case "s3":
		intakeBatches, err = getIntakeBatchesS3(context.Background(), ingestorInputBucket, *ingestorIdentity)
		if err != nil {
			log.Fatal(err)
		}
	case "gs":
		intakeBatches, err = getIntakeBatchesGS(context.Background(), ingestorInputBucket)
		if err != nil {
			log.Fatal(err)
		}
	default:
		log.Fatalf("unknown service %s", ingestorService)
	}

	if err := launchIntake(context.Background(), intakeBatches, ageLimit); err != nil {
		log.Fatal(err)
	}

	log.Printf("found %d ready ingestions", len(intakeBatches))

	var ownValidationBatches []string
	switch ownValidationService {
	case "s3":
		ownValidationBatches, err = getValidationBatchesS3(context.Background(),
			ownValidationInputBucket, *isFirst, intakeBatches, *ownValidationIdentity)
		if err != nil {
			log.Fatal(err)
		}
	case "gs":
		ownValidationBatches, err = getValidationBatchesGS(context.Background(),
			ownValidationInputBucket, *isFirst, intakeBatches)
		if err != nil {
			log.Fatal(err)
		}
	default:
		log.Fatalf("unknown service %s", ownValidationService)
	}

	log.Printf("found %d own validations", len(ownValidationBatches))

	var peerValidationBatches []string
	switch peerValidationService {
	case "s3":
		// These are the peer's validation, and if we were first, then they were
		// not, so negate parseIsFirst's result
		peerValidationBatches, err = getValidationBatchesS3(context.Background(),
			peerValidationInputBucket, !*isFirst, ownValidationBatches, *peerValidationIdentity)
		if err != nil {
			log.Fatal(err)
		}
	case "gs":
		peerValidationBatches, err = getValidationBatchesGS(context.Background(),
			peerValidationInputBucket, !*isFirst, ownValidationBatches)
		if err != nil {
			log.Fatal(err)
		}
	default:
		log.Fatalf("unknown service %s", ownValidationService)
	}

	log.Printf("found %d peer validations", len(peerValidationBatches))

	if err := launchAggregationJob(context.Background(), peerValidationBatches); err != nil {
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

func getIntakeBatchesS3(ctx context.Context, inputBucket string, roleARN string) ([]string, error) {
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
	log.Printf("looking for ready batches in s3://%s as %s", bucket, roleARN)

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
		basename := ingestionBasename(name)
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

func getIntakeBatchesGS(ctx context.Context, inputBucket string) ([]string, error) {
	client, err := storage.NewClient(ctx)
	if err != nil {
		return nil, fmt.Errorf("storage.newClient: %w", err)
	}

	batches := make(map[string]*batch)
	bkt := client.Bucket(inputBucket)
	query := &storage.Query{Prefix: ""}

	log.Printf("looking for ready batches in gs://%s as (ambient service account)", inputBucket)
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
		basename := ingestionBasename(name)
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

func getValidationBatchesGS(ctx context.Context, validationsBucket string, isFirst bool, filter []string) ([]string, error) {
	client, err := storage.NewClient(ctx)
	if err != nil {
		return nil, fmt.Errorf("storage.newClient: %w", err)
	}

	batches := make(map[string]*batch)
	bkt := client.Bucket(validationsBucket)
	query := &storage.Query{Prefix: ""}
	it := bkt.Objects(ctx, query)
	for {
		attrs, err := it.Next()
		if err == iterator.Done {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("iterating on inputBucket objects from %q: %w", validationsBucket, err)
		}
		name := attrs.Name
		basename := validationBasename(name, isFirst)
		if !contains(filter, basename) {
			log.Printf("ignoring batch %s", basename)
			continue
		}
		if batches[basename] == nil {
			batches[basename] = new(batch)
		}
		b := batches[basename]
		if strings.HasSuffix(name, fmt.Sprintf(".validity_%s", index(isFirst))) {
			b.metadata = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".validity_%s.avro", index(isFirst))) {
			b.avro = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".validity_%s.sig", index(isFirst))) {
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

func getValidationBatchesS3(ctx context.Context, validationsBucket string, isFirst bool, filter []string, roleARN string) ([]string, error) {
	parts := strings.SplitN(validationsBucket, "/", 2)
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
		return nil, fmt.Errorf("Unable to list items in bucket %q, %w", validationsBucket, err)
	}
	batches := make(map[string]*batch)
	for _, item := range resp.Contents {
		name := *item.Key
		basename := validationBasename(name, isFirst)
		if !contains(filter, basename) {
			log.Printf("ignoring batch %s", basename)
			continue
		}
		if batches[basename] == nil {
			batches[basename] = new(batch)
		}
		b := batches[basename]
		if strings.HasSuffix(name, fmt.Sprintf(".validity_%s", index(isFirst))) {
			b.metadata = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".validity_%s.avro", index(isFirst))) {
			b.avro = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".validity_%s.sig", index(isFirst))) {
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

func launchAggregationJob(ctx context.Context, readyBatches []string) error {
	if len(readyBatches) == 0 {
		log.Printf("no batches to aggregate")
		return nil
	}

	config, err := rest.InClusterConfig()
	if err != nil {
		return fmt.Errorf("cluster config: %w", err)
	}
	clientset, err := kubernetes.NewForConfig(config)
	if err != nil {
		return fmt.Errorf("clientset: %w", err)
	}
	aggregationID := ""
	var earliestBatch *batchPath
	var latestBatch *batchPath
	batchIDs := []string{}
	batchTimes := []string{}

	for _, batchName := range readyBatches {
		batchPath, err := newBatchPath(batchName)
		if err != nil {
			return err
		}
		batchIDs = append(batchIDs, batchPath.ID)
		batchTimes = append(batchTimes, batchPath.dateString())

		// All batches should have the same aggregation ID?
		if aggregationID != "" && aggregationID != batchPath.aggregationID {
			return fmt.Errorf("found batch with aggregation ID %s, wanted %s", batchPath.aggregationID, aggregationID)
		}
		aggregationID = batchPath.aggregationID

		// Find earliest and latest batch in the aggregation
		if earliestBatch == nil || batchPath.time.Before(earliestBatch.time) {
			earliestBatch = &batchPath
		}
		if latestBatch == nil || batchPath.time.After(latestBatch.time) {
			latestBatch = &batchPath
		}
	}

	args := []string{
		"aggregate",
		"--aggregation-id", aggregationID,
		"--aggregation-start", earliestBatch.dateString(),
		"--aggregation-end", latestBatch.dateString(),
	}
	for _, batchID := range batchIDs {
		args = append(args, "--batch-id")
		args = append(args, batchID)
	}
	for _, batchTime := range batchTimes {
		args = append(args, "--batch-time")
		args = append(args, batchTime)
	}

	jobName := strings.ReplaceAll(fmt.Sprintf("a-%s", earliestBatch.dateString()), "/", "-")

	log.Printf("starting aggregation job with args %s", args)

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
							Image:           *facilitatorImage,
							ImagePullPolicy: "Always",
							EnvFrom: []corev1.EnvFromSource{
								{
									ConfigMapRef: &corev1.ConfigMapEnvSource{
										LocalObjectReference: corev1.LocalObjectReference{
											Name: *aggregateConfigMap,
										},
									},
								},
							},
							Env: []corev1.EnvVar{
								{
									Name: "BATCH_SIGNING_PRIVATE_KEY",
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
		if errors.IsAlreadyExists(err) {
			log.Printf("skipping %q because a job for it already exists (err %T = %#v)",
				jobName, err, err)
			return nil
		}
		return fmt.Errorf("creating job: %w", err)
	}
	log.Printf("Created job %q: %#v", jobName, createdJob.ObjectMeta.UID)

	return nil
}

func launchIntake(ctx context.Context, readyBatches []string, ageLimit time.Duration) error {
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
		if err := startIntakeJob(ctx, clientset, batchName, ageLimit); err != nil {
			return fmt.Errorf("starting job for batch %q: %w", batchName, err)
		}
	}
	return nil
}

func startIntakeJob(
	ctx context.Context,
	clientset *kubernetes.Clientset,
	batchName string,
	ageLimit time.Duration,
) error {
	batchPath, err := newBatchPath(batchName)
	if err != nil {
		return err
	}

	age := time.Now().Sub(batchPath.time)
	if age > ageLimit {
		log.Printf("skipping batch %q because it is too old (%s)", batchName, age)
		return nil
	}

	jobName := fmt.Sprintf("i-batch-%s", batchPath.ID)

	args := []string{
		"intake-batch",
		"--aggregation-id", batchPath.aggregationID,
		"--batch-id", batchPath.ID,
		"--date", batchPath.dateString(),
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
							Image:           *facilitatorImage,
							ImagePullPolicy: "Always",
							Resources: corev1.ResourceRequirements{
								Requests: corev1.ResourceList{
									corev1.ResourceMemory: resource.MustParse("500Mi"),
									corev1.ResourceCPU:    resource.MustParse("0.5"),
								},
								Limits: corev1.ResourceList{
									corev1.ResourceMemory: resource.MustParse("550Mi"),
									corev1.ResourceCPU:    resource.MustParse("0.7"),
								},
							},
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
									Name: "BATCH_SIGNING_PRIVATE_KEY",
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
		if errors.IsAlreadyExists(err) {
			log.Printf("skipping %q because a job for it already exists", batchName)
			return nil
		}
		return fmt.Errorf("creating job: %w", err)
	}
	log.Printf("Created job %q: %#v", jobName, createdJob.ObjectMeta.UID)

	return nil
}
