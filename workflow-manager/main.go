// workflow-manager looks for batches to be processed from an input bucket,
// and spins up `facilitator intake-batch` jobs to process those batches.
//
// It also looks for batches that have been intake'd, and spins up
// `facilitator aggregate` jobs to aggregate them and write to a portal bucket.
package main

import (
	"context"
	"flag"
	"fmt"
	"io/ioutil"
	"log"
	"net/http"
	"os"
	"regexp"
	"sort"
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

// BuildInfo is generated at build time - see the Dockerfile.
var BuildInfo string

type batchPath struct {
	aggregationID  string
	dateComponents []string
	ID             string
	time           time.Time
	metadata       bool
	avro           bool
	sig            bool
}

type batchPathList []*batchPath

func (bpl batchPathList) Len() int {
	return len(bpl)
}

func (bpl batchPathList) Less(i, j int) bool {
	return bpl[i].time.Before(bpl[j].time)
}

func (bpl batchPathList) Swap(i, j int) {
	bpl[i], bpl[j] = bpl[j], bpl[i]
}

func newBatchPath(batchName string) (*batchPath, error) {
	// batchName is like "kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771"
	pathComponents := strings.Split(batchName, "/")
	batchID := pathComponents[len(pathComponents)-1]
	aggregationID := pathComponents[0]
	batchDate := pathComponents[1 : len(pathComponents)-1]

	if len(batchDate) != 5 {
		return nil, fmt.Errorf("malformed date in %q. Expected 5 date components, got %d", batchName, len(batchDate))
	}

	var dateComponents []int
	for _, c := range batchDate {
		parsed, err := strconv.ParseInt(c, 10, 64)
		if err != nil {
			return nil, fmt.Errorf("parsing date component %q in %q: %w", c, batchName, err)
		}
		dateComponents = append(dateComponents, int(parsed))
	}
	batchTime := time.Date(dateComponents[0], time.Month(dateComponents[1]),
		dateComponents[2], dateComponents[3], dateComponents[4], 0, 0, time.UTC)

	return &batchPath{
		aggregationID:  aggregationID,
		dateComponents: batchDate,
		ID:             batchID,
		time:           batchTime,
	}, nil
}

func (b *batchPath) String() string {
	return fmt.Sprintf("{%s %s %s files:%d%d%d}", b.aggregationID, b.dateComponents, b.ID, index(!b.metadata), index(!b.avro), index(!b.sig))
}

func (b *batchPath) path() string {
	return strings.Join([]string{b.aggregationID, b.dateString(), b.ID}, "/")
}

func (b *batchPath) dateString() string {
	return strings.Join(b.dateComponents, "/")
}

// basename returns s, with any type suffixes stripped off. The type suffixes are determined by
// `infix`, which is one of "batch", "validity_0", or "validity_1".
func basename(s string, infix string) string {
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s", infix))
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s.avro", infix))
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s.sig", infix))
	return s
}

// index returns the appropriate int to use in construction of filenames
// based on whether the file creator was the "first" aka PHA server.
func index(isFirst bool) int {
	if isFirst {
		return 0
	}
	return 1
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

var k8sNS = flag.String("k8s-namespace", "", "Kubernetes namespace")
var isFirst = flag.Bool("is-first", false, "Whether this set of servers is \"first\", aka PHA servers")
var maxAge = flag.String("intake-max-age", "1h", "Max age (in Go duration format) for intake batches to be worth processing.")
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
var aggregationPeriod = flag.String("aggregation-period", "3h", "How much time each aggregation covers")
var gracePeriod = flag.String("grace-period", "1h", "Wait this amount of time after the end of an aggregation timeslice to run the aggregation")

func main() {
	log.Printf("starting %s version %s. Args: %s", os.Args[0], BuildInfo, os.Args[1:])
	flag.Parse()

	ownValidationBucket, err := newBucket(*ownValidationInput, *ownValidationIdentity)
	if err != nil {
		log.Fatalf("--ingestor-input: %s", err)
	}
	peerValidationBucket, err := newBucket(*peerValidationInput, *peerValidationIdentity)
	if err != nil {
		log.Fatalf("--ingestor-input: %s", err)
	}
	intakeBucket, err := newBucket(*ingestorInput, *ingestorIdentity)
	if err != nil {
		log.Fatalf("--ingestor-input: %s", err)
	}

	if *intakeConfigMap == "" || *aggregateConfigMap == "" {
		log.Fatal("--intake-batch-config-map and --aggregate-config-map are required")
	}
	if *facilitatorImage == "" {
		log.Fatal("--facilitator-image is required")
	}

	maxAgeParsed, err := time.ParseDuration(*maxAge)
	if err != nil {
		log.Fatalf("--max-age: %s", err)
	}

	gracePeriodParsed, err := time.ParseDuration(*gracePeriod)
	if err != nil {
		log.Fatalf("--grace-period: %s", err)
	}

	aggregationPeriodParsed, err := time.ParseDuration(*aggregationPeriod)
	if err != nil {
		log.Fatalf("--aggregation-time-slice: %s", err)
	}

	intakeFiles, err := intakeBucket.listFiles(context.Background())
	if err != nil {
		log.Fatal(err)
	}

	intakeBatches, err := readyBatches(intakeFiles, "batch")
	if err != nil {
		log.Fatal(err)
	}

	currentIntakeBatches := withinInterval(intakeBatches, interval{
		begin: time.Now().Add(-maxAgeParsed),
		end:   time.Now().Add(24 * time.Hour),
	})
	log.Printf("skipping %d batches as too old", len(intakeBatches)-len(currentIntakeBatches))

	if err := launchIntake(context.Background(), currentIntakeBatches, maxAgeParsed); err != nil {
		log.Fatal(err)
	}

	ownValidationFiles, err := ownValidationBucket.listFiles(context.Background())
	if err != nil {
		log.Fatal(err)
	}

	ownValidityInfix := fmt.Sprintf("validity_%d", index(*isFirst))
	ownValidationBatches, err := readyBatches(ownValidationFiles, ownValidityInfix)
	if err != nil {
		log.Fatal(err)
	}

	log.Printf("found %d own validations", len(ownValidationBatches))

	peerValidationFiles, err := peerValidationBucket.listFiles(context.Background())
	if err != nil {
		log.Fatal(err)
	}

	peerValidityInfix := fmt.Sprintf("validity_%d", index(!*isFirst))
	peerValidationBatches, err := readyBatches(peerValidationFiles, peerValidityInfix)
	if err != nil {
		log.Fatal(err)
	}

	log.Printf("found %d peer validations", len(peerValidationBatches))

	// TODO: Check that both peer and own have something.
	var aggregationBatches = peerValidationBatches
	interval := aggregationInterval(aggregationPeriodParsed, gracePeriodParsed)
	log.Printf("looking for batches to aggregate in interval %s", interval)
	aggregationBatches = withinInterval(aggregationBatches, interval)
	aggregationMap := groupByAggregationID(aggregationBatches)

	if err := launchAggregationJobs(context.Background(), aggregationMap, interval); err != nil {
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

type bucket struct {
	service string // "s3" or "gs"
	// bucketName includes the region for S3
	bucketName string
	identity   string
}

func newBucket(bucketURL, identity string) (*bucket, error) {
	if bucketURL == "" {
		log.Fatalf("empty bucket URL")
	}
	if !strings.HasPrefix(bucketURL, "s3://") && !strings.HasPrefix(bucketURL, "gs://") {
		return nil, fmt.Errorf("invalid bucket %q with identity %q", bucketURL, identity)
	}
	if strings.HasPrefix(bucketURL, "gs://") && identity != "" {
		return nil, fmt.Errorf("workflow-manager doesn't support alternate identities (%s) for gs:// bucket (%q)",
			identity, bucketURL)
	}
	return &bucket{
		service:    bucketURL[0:2],
		bucketName: bucketURL[5:],
		identity:   identity,
	}, nil
}

func (b *bucket) listFiles(ctx context.Context) ([]string, error) {
	switch b.service {
	case "s3":
		return b.listFilesS3(ctx)
	case "gs":
		return b.listFilesGS(ctx)
	default:
		return nil, fmt.Errorf("invalid storage service %q", b.service)
	}
}

func (b *bucket) listFilesS3(ctx context.Context) ([]string, error) {
	parts := strings.SplitN(b.bucketName, "/", 2)
	if len(parts) != 2 {
		return nil, fmt.Errorf("invalid S3 bucket name %q", b.bucketName)
	}
	region := parts[0]
	bucket := parts[1]
	sess, err := aws_session.NewSession()
	if err != nil {
		return nil, fmt.Errorf("making AWS session: %w", err)
	}

	arnComponents := strings.Split(b.identity, ":")
	if arnComponents[0] != "arn" {
		return nil, fmt.Errorf("invalid AWS identity %q. Must start with \"arn:\"", b.identity)
	}
	if len(arnComponents) != 6 {
		return nil, fmt.Errorf("invalid ARN: %q", b.identity)
	}
	audience := fmt.Sprintf("sts.amazonaws.com/%s", arnComponents[4])

	stsSTS := sts.New(sess)
	roleSessionName := ""
	roleProvider := stscreds.NewWebIdentityRoleProviderWithToken(
		stsSTS, b.identity, roleSessionName, tokenFetcher{audience})

	credentials := credentials.NewCredentials(roleProvider)
	log.Printf("listing files in s3://%s as %s", bucket, b.identity)

	config := aws.NewConfig().
		WithRegion(region).
		WithCredentials(credentials)
	svc := s3.New(sess, config)
	var output []string
	var nextContinuationToken string = ""
	for {
		input := &s3.ListObjectsV2Input{
			// We choose a lower number than the default max of 1000 to ensure we exercise the
			// cursoring case regularly.
			MaxKeys: aws.Int64(100),
			Bucket:  aws.String(bucket),
		}
		if nextContinuationToken != "" {
			input.ContinuationToken = &nextContinuationToken
		}
		resp, err := svc.ListObjectsV2(input)
		if err != nil {
			return nil, fmt.Errorf("Unable to list items in bucket %q, %w", b.bucketName, err)
		}
		for _, item := range resp.Contents {
			output = append(output, *item.Key)
		}
		if !*resp.IsTruncated {
			break
		}
		nextContinuationToken = *resp.NextContinuationToken
	}
	return output, nil
}

func (b *bucket) listFilesGS(ctx context.Context) ([]string, error) {
	if b.identity != "" {
		return nil, fmt.Errorf("workflow-manager doesn't support non-default identity %q for GS bucket %q", b.identity, b.bucketName)
	}
	client, err := storage.NewClient(ctx)
	if err != nil {
		return nil, fmt.Errorf("storage.newClient: %w", err)
	}

	bkt := client.Bucket(b.bucketName)
	query := &storage.Query{Prefix: ""}

	log.Printf("looking for ready batches in gs://%s as (ambient service account)", b.bucketName)
	var output []string
	it := bkt.Objects(ctx, query)
	for {
		attrs, err := it.Next()
		if err == iterator.Done {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("iterating on inputBucket objects from %q: %w", b.bucketName, err)
		}
		output = append(output, attrs.Name)
	}
	return output, nil
}

func readyBatches(files []string, infix string) ([]*batchPath, error) {
	batches := make(map[string]*batchPath)
	for _, name := range files {
		basename := basename(name, infix)
		b := batches[basename]
		var err error
		if b == nil {
			b, err = newBatchPath(basename)
			if err != nil {
				return nil, err
			}
			batches[basename] = b
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s", infix)) {
			b.metadata = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s.avro", infix)) {
			b.avro = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s.sig", infix)) {
			b.sig = true
		}
	}

	var output []*batchPath
	for _, v := range batches {
		output = append(output, v)
	}
	sort.Sort(batchPathList(output))

	return output, nil
}

// interval represents a half-open interval of time.
// It includes `begin` and excludes `end`.
type interval struct {
	begin time.Time
	end   time.Time
}

func (inter interval) String() string {
	return fmt.Sprintf("%s to %s", fmtTime(inter.begin), fmtTime(inter.end))
}

// fmtTime returns the input time in the same style expected by facilitator/lib.rs,
// currently "%Y/%m/%d/%H/%M"
func fmtTime(t time.Time) string {
	return t.Format("2006/01/02/15/04")
}

// idForJobName generates a job name-safe string from an aggregationID.
// Remove characters that aren't valid in DNS names, and also restrict
// the length so we don't go over the limit of 63 characters.
func idForJobName(aggregationID string) string {
	re := regexp.MustCompile("[^A-Za-z0-9-]")
	idForJobName := re.ReplaceAllLiteralString(aggregationID, "-")
	if len(idForJobName) > 30 {
		idForJobName = idForJobName[:30]
	}
	return idForJobName
}

// aggregationInterval calculates the interval we want to run an aggregation for, if any.
// That is whatever interval is `gracePeriod` earlier than now and aligned on multiples
// of `aggregationPeriod` (relative to the zero time).
func aggregationInterval(aggregationPeriod, gracePeriod time.Duration) interval {
	var output interval
	output.end = time.Now().Add(-gracePeriod).Truncate(aggregationPeriod)
	output.begin = output.end.Add(-aggregationPeriod)
	return output
}

// withinInterval returns the subset of `batchPath`s that are within the given interval.
func withinInterval(batches []*batchPath, inter interval) []*batchPath {
	var output []*batchPath
	for _, bp := range batches {
		// We use Before twice rather than Before and after, because Before is <,
		// and After is >, but we are processing a half-open interval so we need
		// >= and <.
		if !bp.time.Before(inter.begin) && bp.time.Before(inter.end) {
			output = append(output, bp)
		}
	}
	return output
}

type aggregationMap map[string][]*batchPath

func groupByAggregationID(batches []*batchPath) aggregationMap {
	output := make(aggregationMap)
	for _, v := range batches {
		output[v.aggregationID] = append(output[v.aggregationID], v)
	}
	return output
}

func launchAggregationJobs(ctx context.Context, batchesByID aggregationMap, inter interval) error {
	if len(batchesByID) == 0 {
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

	for _, readyBatches := range batchesByID {
		aggregationID := readyBatches[0].aggregationID

		args := []string{
			"aggregate",
			"--aggregation-id", aggregationID,
			"--aggregation-start", fmtTime(inter.begin),
			"--aggregation-end", fmtTime(inter.end),
		}
		for _, batchPath := range readyBatches {
			args = append(args, "--batch-id")
			args = append(args, batchPath.ID)
			args = append(args, "--batch-time")
			args = append(args, batchPath.dateString())

			// All batches should have the same aggregation ID?
			if aggregationID != batchPath.aggregationID {
				return fmt.Errorf("found batch with aggregation ID %s, wanted %s", batchPath.aggregationID, aggregationID)
			}
		}

		jobName := fmt.Sprintf("a-%s-%s", idForJobName(aggregationID), strings.ReplaceAll(fmtTime(inter.begin), "/", "-"))

		log.Printf("starting aggregation job %s (interval %s) with args %s", jobName, inter, args)

		var one int32 = 1
		job := &batchv1.Job{
			ObjectMeta: metav1.ObjectMeta{
				Name:      jobName,
				Namespace: *k8sNS,
			},
			Spec: batchv1.JobSpec{
				BackoffLimit: &one,
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
				continue
			}
			log.Printf("creating job: %s", err)
			continue
		}
		log.Printf("Created job %q: %s", jobName, createdJob.ObjectMeta.UID)
	}

	return nil
}

func launchIntake(ctx context.Context, readyBatches []*batchPath, ageLimit time.Duration) error {
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
	for _, batch := range readyBatches {
		if err := startIntakeJob(ctx, clientset, batch, ageLimit); err != nil {
			return fmt.Errorf("starting job for batch %s: %w", batch, err)
		}
	}
	return nil
}

func startIntakeJob(
	ctx context.Context,
	clientset *kubernetes.Clientset,
	batchPath *batchPath,
	ageLimit time.Duration,
) error {
	age := time.Now().Sub(batchPath.time)
	if age > ageLimit {
		log.Printf("skipping batch %s because it is too old (%s)", batchPath, age)
		return nil
	}

	jobName := fmt.Sprintf("i-%s-%s",
		idForJobName(batchPath.aggregationID),
		strings.ReplaceAll(fmtTime(batchPath.time), "/", "-"))

	args := []string{
		"intake-batch",
		"--aggregation-id", batchPath.aggregationID,
		"--batch-id", batchPath.ID,
		"--date", batchPath.dateString(),
	}
	log.Printf("starting job for batch %s with args %s", batchPath, args)
	var one int32 = 1
	job := &batchv1.Job{
		ObjectMeta: metav1.ObjectMeta{
			Name:      jobName,
			Namespace: *k8sNS,
		},
		Spec: batchv1.JobSpec{
			BackoffLimit: &one,
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
			log.Printf("skipping %s because a job for it already exists", batchPath)
			return nil
		}
		return fmt.Errorf("creating job: %w", err)
	}
	log.Printf("Created job %q: %s", jobName, createdJob.ObjectMeta.UID)

	return nil
}
