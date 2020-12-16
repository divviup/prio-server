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
	"strings"
	"time"

	"cloud.google.com/go/storage"
	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/arn"
	"github.com/aws/aws-sdk-go/aws/credentials"
	"github.com/aws/aws-sdk-go/aws/credentials/stscreds"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/letsencrypt/prio-server/workflow-manager/batchpath"
	wferror "github.com/letsencrypt/prio-server/workflow-manager/errors"
	"github.com/letsencrypt/prio-server/workflow-manager/monitor"
	"github.com/letsencrypt/prio-server/workflow-manager/retry"
	"github.com/letsencrypt/prio-server/workflow-manager/utils"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"github.com/prometheus/client_golang/prometheus/push"
	"google.golang.org/api/iterator"
	"google.golang.org/api/sts/v1"
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

var k8sNS = flag.String("k8s-namespace", "", "Kubernetes namespace")
var isFirst = flag.Bool("is-first", false, "Whether this set of servers is \"first\", aka PHA servers")
var maxAge = flag.String("intake-max-age", "1h", "Max age (in Go duration format) for intake batches to be worth processing.")
var k8sServiceAccount = flag.String("k8s-service-account", "", "Kubernetes service account for intake and aggregate jobs")
var bskSecretSelector = flag.String("bsk-secret-selector", "", "Label selector of k8s secret for batch signing key")
var pdksSecretSelector = flag.String("pdks-secret-selector", "", "Label selector of k8s secret for packet decrypt keys")
var gcpServiceAccountKeyFileSecretName = flag.String("gcp-service-account-key-file-secret-name", "", "Name of k8s secret for default GCP service account key file")
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
var pushGateway = flag.String("push-gateway", "", "Set this to the gateway to use with prometheus. If left empty, workflow-manager will not use prometheus.")

// monitoring things
var (
	intakesStarted      monitor.CounterMonitor = &monitor.NoopCounter{}
	aggregationsStarted monitor.CounterMonitor = &monitor.NoopCounter{}
)

func main() {
	log.Printf("starting %s version %s. Args: %s", os.Args[0], BuildInfo, os.Args[1:])
	flag.Parse()

	if *pushGateway != "" {
		push.New(*pushGateway, "workflow-manager").Gatherer(prometheus.DefaultGatherer).Push()
		intakesStarted = promauto.NewCounter(prometheus.CounterOpts{
			Name: "intake_jobs_started",
			Help: "The number of intake-batch jobs successfully started",
		})

		aggregationsStarted = promauto.NewCounter(prometheus.CounterOpts{
			Name: "aggregation_jobs_started",
			Help: "The number of aggregate jobs successfully started",
		})
	}

	ownValidationBucket, err := bucket.New(*ownValidationInput, *ownValidationIdentity)
	if err != nil {
		log.Fatalf("--ingestor-input: %s", err)
	}
	peerValidationBucket, err := bucket.New(*peerValidationInput, *peerValidationIdentity)
	if err != nil {
		log.Fatalf("--ingestor-input: %s", err)
	}
	intakeBucket, err := bucket.New(*ingestorInput, *ingestorIdentity)
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

	intakeFiles, err := intakeBucket.ListFiles(context.Background())
	if err != nil {
		log.Fatal(err)
	}

	intakeBatches, err := batchpath.ReadyBatches(intakeFiles, "batch")
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

	ownValidationFiles, err := ownValidationBucket.ListFiles(context.Background())
	if err != nil {
		log.Fatal(err)
	}

	ownValidityInfix := fmt.Sprintf("validity_%d", utils.Index(*isFirst))
	ownValidationBatches, err := batchpath.ReadyBatches(ownValidationFiles, ownValidityInfix)
	if err != nil {
		log.Fatal(err)
	}

	log.Printf("found %d own validations", len(ownValidationBatches))

	peerValidationFiles, err := peerValidationBucket.ListFiles(context.Background())
	if err != nil {
		log.Fatal(err)
	}

	peerValidityInfix := fmt.Sprintf("validity_%d", utils.Index(!*isFirst))
	peerValidationBatches, err := batchpath.ReadyBatches(peerValidationFiles, peerValidityInfix)
	if err != nil {
		log.Fatal(err)
	}

	log.Printf("found %d peer validations", len(peerValidationBatches))

	// Take the intersection of the sets of own validations and peer validations to get the list of
	// batches we can aggregate.
	// Go doesn't have sets, so we have to use a map[string]bool. We use the batch ID as the key to
	// the set, because batchPath is not a valid map key type, and using a *batchPath wouldn't give
	// us the lookup semantics we want.
	ownValidationsSet := map[string]bool{}
	for _, ownValidationBatch := range ownValidationBatches {
		ownValidationsSet[ownValidationBatch.ID] = true
	}
	aggregationBatches := batchpath.List{}
	for _, peerValidationBatch := range peerValidationBatches {
		if _, ok := ownValidationsSet[peerValidationBatch.ID]; ok {
			aggregationBatches = append(aggregationBatches, peerValidationBatch)
		}
	}

	interval := aggregationInterval(aggregationPeriodParsed, gracePeriodParsed)
	log.Printf("looking for batches to aggregate in interval %s", interval)
	aggregationBatches = withinInterval(aggregationBatches, interval)
	aggregationMap := groupByAggregationID(aggregationBatches)
	r := retry.Retry{
		Identifier: "Aggregation",
		Retryable: func() error {
			return launchAggregationJobs(context.Background(), aggregationMap, interval)
		},
		ShouldRequeue: wferror.IsTransientErr,
		// 5 is an arbitrary number... it seemed right
		MaxTries: 5,
		// No need to wait between retries
		TimeBetweenTries: 0,
	}

	if err := r.Start(); err != nil {
		log.Fatal(err)
	} else {
		aggregationsStarted.Inc()
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

func webIDP(sess *aws_session.Session, identity string) (*credentials.Credentials, error) {
	parsed, err := arn.Parse(identity)
	if err != nil {
		return nil, err
	}
	audience := fmt.Sprintf("sts.amazonaws.com/%s", parsed.AccountID)

	stsSTS := sts.New(sess)
	roleSessionName := ""
	roleProvider := stscreds.NewWebIdentityRoleProviderWithToken(
		stsSTS, identity, roleSessionName, tokenFetcher{audience})

	return credentials.NewCredentials(roleProvider), nil
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

	log.Printf("listing files in s3://%s as %q", bucket, b.identity)
	config := aws.NewConfig().
		WithRegion(region)
	if b.identity != "" {
		creds, err := webIDP(sess, b.identity)
		if err != nil {
			return nil, err
		}
		config = config.WithCredentials(creds)
	}
	svc := s3.New(sess, config)
	var output []string
	var nextContinuationToken = ""
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

// jobNameForBatchPath generates a name for the Kubernetes job that will intake
// the provided batch. The name will incorporate the aggregation ID, batch UUID
// and batch timestamp while being a legal Kubernetes job name.
func intakeJobNameForBatchPath(path *batchpath.BatchPath) string {
	// Kubernetes job names must be valid DNS identifiers, which means they are
	// limited to 63 characters in length and also what characters they may
	// contain. Intake job names are like:
	// i-<aggregation name fragment>-<batch UUID fragment>-<batch timestamp>
	// The batch timestamp is 16 characters, and the 'i' and '-'es take up
	// another 4, leaving 43. We take the '-'es out of the UUID and use half of
	// it, hoping that this plus the date will provide enough entropy to avoid
	// collisions. Half a UUID is 16 characters, leaving 27 for the aggregation
	// ID fragment.
	// For example, we might get:
	// i-com-apple-EN-verylongnameth-0f0f0f0f0f0f0f0f-2006-01-02-15-04
	return fmt.Sprintf("i-%s-%s-%s",
		aggregationJobNameFragment(path.AggregationID, 27),
		strings.ReplaceAll(path.ID, "-", "")[:16],
		strings.ReplaceAll(fmtTime(path.Time), "/", "-"))
}

// aggregationJobNameFragment generates a job name-safe string from an
// aggregationID.
// Remove characters that aren't valid in DNS names, and also restrict
// the length so we don't go over the specified limit of characters.
func aggregationJobNameFragment(aggregationID string, maxLength int) string {
	re := regexp.MustCompile("[^A-Za-z0-9-]")
	idForJobName := re.ReplaceAllLiteralString(aggregationID, "-")
	if len(idForJobName) > maxLength {
		idForJobName = idForJobName[:maxLength]
	}
	idForJobName = strings.ToLower(idForJobName)
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
func withinInterval(batches batchpath.List, inter interval) batchpath.List {
	var output batchpath.List
	for _, bp := range batches {
		// We use Before twice rather than Before and after, because Before is <,
		// and After is >, but we are processing a half-open interval so we need
		// >= and <.
		if !bp.Time.Before(inter.begin) && bp.Time.Before(inter.end) {
			output = append(output, bp)
		}
	}
	return output
}

type aggregationMap map[string]batchpath.List

func groupByAggregationID(batches batchpath.List) aggregationMap {
	output := make(aggregationMap)
	for _, v := range batches {
		output[v.AggregationID] = append(output[v.AggregationID], v)
	}
	return output
}

func secretVolumesAndMounts() ([]corev1.Volume, []corev1.VolumeMount) {
	volumes := []corev1.Volume{}
	volumeMounts := []corev1.VolumeMount{}
	if *gcpServiceAccountKeyFileSecretName != "" {
		volumes = append(volumes, corev1.Volume{
			Name: "default-gcp-sa-key-file",
			VolumeSource: corev1.VolumeSource{
				Secret: &corev1.SecretVolumeSource{
					SecretName: *gcpServiceAccountKeyFileSecretName,
				},
			},
		})

		volumeMounts = append(volumeMounts, corev1.VolumeMount{
			Name:      "default-gcp-sa-key-file",
			MountPath: "/etc/secrets",
			ReadOnly:  true,
		})
	}

	return volumes, volumeMounts
}

func getLatestBatchSigningKey(ctx context.Context, client *kubernetes.Clientset) (*corev1.Secret, error) {
	secrets, err := getSecretsWithLabel(ctx, client, *bskSecretSelector)
	if err != nil {
		return nil, fmt.Errorf("bsks: %w", err)
	}

	if len(secrets) == 0 {
		return nil, fmt.Errorf("secrets did not exist. %w", err)
	}

	// secrets with descending (older) creation dates
	sort.Slice(secrets, func(i, j int) bool {
		item1 := secrets[i]
		item2 := secrets[j]

		time1 := item1.GetCreationTimestamp().Time
		time2 := item2.GetCreationTimestamp().Time

		return time1.After(time2)
	})

	return &secrets[0], nil
}

func getAvailablePacketDecryptionKeys(ctx context.Context, client *kubernetes.Clientset) ([]corev1.Secret, error) {
	return getSecretsWithLabel(ctx, client, *pdksSecretSelector)
}

func getSecretsWithLabel(ctx context.Context, client *kubernetes.Clientset, labelSelector string) ([]corev1.Secret, error) {
	secrets, err := client.CoreV1().Secrets(*k8sNS).List(ctx, metav1.ListOptions{LabelSelector: labelSelector})
	if err != nil {
		return nil, fmt.Errorf("secrets with label %s: %w", labelSelector, err)
	}

	if secrets.Items == nil {
		return nil, fmt.Errorf("pdks were nil after retrieving them from k8s")
	}

	return secrets.Items, nil
}

func getBSKAndPDKs(ctx context.Context, client *kubernetes.Clientset) (*corev1.Secret, *string, error) {
	bsk, err := getLatestBatchSigningKey(context.Background(), client)
	if err != nil {
		return nil, nil, fmt.Errorf("bsk: %w", err)
	}

	pdks, err := getAvailablePacketDecryptionKeys(context.Background(), client)
	if err != nil {
		return nil, nil, fmt.Errorf("pdks: %w", err)
	}

	pdksStringArray := make([]string, len(pdks))
	for _, pdk := range pdks {
		data := pdk.Data["secret_key"]
		if len(data) == 0 {
			return nil, nil, fmt.Errorf("pdk did not contain secret_key")
		}
		pdksStringArray = append(pdksStringArray, string(data))
	}

	pdksDelimited := strings.Join(pdksStringArray, ",")

	return bsk, &pdksDelimited, nil
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

	bsk, pdks, err := getBSKAndPDKs(ctx, clientset)
	if err != nil {
		return fmt.Errorf("bsk and pdks: %w", err)
	}

	for _, readyBatches := range batchesByID {
		aggregationID := readyBatches[0].AggregationID

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
			args = append(args, batchPath.DateString())

			// All batches should have the same aggregation ID?
			if aggregationID != batchPath.AggregationID {
				return fmt.Errorf("found batch with aggregation ID %s, wanted %s", batchPath.AggregationID, aggregationID)
			}
		}

		jobName := fmt.Sprintf("a-%s-%s", aggregationJobNameFragment(aggregationID, 30), strings.ReplaceAll(fmtTime(inter.begin), "/", "-"))

		log.Printf("starting aggregation job %s (interval %s) with args %s", jobName, inter, args)

		var one int32 = 1
		volumes, volumeMounts := secretVolumesAndMounts()
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
						Volumes:            volumes,
						Containers: []corev1.Container{
							{
								Args:            args,
								Name:            "facile-container",
								Image:           *facilitatorImage,
								ImagePullPolicy: "IfNotPresent",
								VolumeMounts:    volumeMounts,
								Resources: corev1.ResourceRequirements{
									Requests: corev1.ResourceList{
										corev1.ResourceMemory: resource.MustParse("500Mi"),
										corev1.ResourceCPU:    resource.MustParse("1.0"),
									},
									Limits: corev1.ResourceList{
										corev1.ResourceMemory: resource.MustParse("550Mi"),
										corev1.ResourceCPU:    resource.MustParse("1.5"),
									},
								},
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
													Name: bsk.Name,
												},
												Key: "secret_key",
											},
										},
									},
									{
										Name:  "PACKET_DECRYPTION_KEYS",
										Value: *pdks,
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

func launchIntake(ctx context.Context, readyBatches batchpath.List, ageLimit time.Duration) error {
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

	log.Printf("starting %d jobs", readyBatches.Len())
	for _, batch := range readyBatches {
		r := retry.Retry{
			Identifier: batch.String(),
			Retryable: func() error {
				return startIntakeJob(ctx, clientset, batch, ageLimit)
			},
			ShouldRequeue: wferror.IsTransientErr,
			// 5 Seems right, we're retrying the same amount for this as we do for the aggregation jobs
			MaxTries: 5,
			// lets not wait between retries
			TimeBetweenTries: 0,
		}

		if err := r.Start(); err != nil {
			return fmt.Errorf("starting job for batch %s: %w", batch, err)
		}
		intakesStarted.Inc()
	}
	return nil
}

func startIntakeJob(
	ctx context.Context,
	clientset *kubernetes.Clientset,
	batchPath *batchpath.BatchPath,
	ageLimit time.Duration,
	batchSigningKey *corev1.Secret,
	packetDecryptionDelimited string,
) error {
	age := time.Now().Sub(batchPath.Time)
	if age > ageLimit {
		log.Printf("skipping batch %s because it is too old (%s)", batchPath, age)
		return nil
	}

	jobName := intakeJobNameForBatchPath(batchPath)
	args := []string{
		"intake-batch",
		"--aggregation-id", batchPath.AggregationID,
		"--batch-id", batchPath.ID,
		"--date", batchPath.DateString(),
	}
	log.Printf("starting job for batch %s with args %s", batchPath, args)

	volumes, volumeMounts := secretVolumesAndMounts()

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
					Volumes:            volumes,
					Containers: []corev1.Container{
						{
							Args:            args,
							Name:            "facile-container",
							Image:           *facilitatorImage,
							ImagePullPolicy: "IfNotPresent",
							VolumeMounts:    volumeMounts,
							Resources: corev1.ResourceRequirements{
								Requests: corev1.ResourceList{
									corev1.ResourceMemory: resource.MustParse("500Mi"),
									corev1.ResourceCPU:    resource.MustParse("1.0"),
								},
								Limits: corev1.ResourceList{
									corev1.ResourceMemory: resource.MustParse("550Mi"),
									corev1.ResourceCPU:    resource.MustParse("1.5"),
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
												Name: batchSigningKey.Name,
											},
											Key: "secret_key",
										},
									},
								},
								{
									Name:  "PACKET_DECRYPTION_KEYS",
									Value: packetDecryptionDelimited,
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
