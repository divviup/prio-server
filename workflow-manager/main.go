// workflow-manager looks for batches to be processed from an input bucket,
// and schedules intake-batch tasks for intake-batch-workers to process those
// batches.
//
// It also looks for batches that have been intake'd, and schedules aggregate
// tasks.
package main

import (
	"flag"
	"fmt"
	"log"
	"os"
	"strings"
	"time"

	"github.com/letsencrypt/prio-server/workflow-manager/batchpath"
	"github.com/letsencrypt/prio-server/workflow-manager/bucket"
	"github.com/letsencrypt/prio-server/workflow-manager/monitor"
	"github.com/letsencrypt/prio-server/workflow-manager/task"
	"github.com/letsencrypt/prio-server/workflow-manager/utils"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"github.com/prometheus/client_golang/prometheus/push"
)

// BuildInfo is generated at build time - see the Dockerfile.
var BuildInfo string

var k8sNS = flag.String("k8s-namespace", "", "Kubernetes namespace")
var ingestorLabel = flag.String("ingestor-label", "", "Label of ingestion server")
var isFirst = flag.Bool("is-first", false, "Whether this set of servers is \"first\", aka PHA servers")
var maxAge = flag.String("intake-max-age", "1h", "Max age (in Go duration format) for intake batches to be worth processing.")
var ingestorInput = flag.String("ingestor-input", "", "Bucket for input from ingestor (s3:// or gs://) (Required)")
var ingestorIdentity = flag.String("ingestor-identity", "", "Identity to use with ingestor bucket (Required for S3)")
var ownValidationInput = flag.String("own-validation-input", "", "Bucket for input of validation batches from self (s3:// or gs://) (required)")
var ownValidationIdentity = flag.String("own-validation-identity", "", "Identity to use with own validation bucket (Required for S3)")
var peerValidationInput = flag.String("peer-validation-input", "", "Bucket for input of validation batches from peer (s3:// or gs://) (required)")
var peerValidationIdentity = flag.String("peer-validation-identity", "", "Identity to use with peer validation bucket (Required for S3)")
var aggregationPeriod = flag.String("aggregation-period", "3h", "How much time each aggregation covers")
var gracePeriod = flag.String("grace-period", "1h", "Wait this amount of time after the end of an aggregation timeslice to run the aggregation")
var pushGateway = flag.String("push-gateway", "", "Set this to the gateway to use with prometheus. If left empty, workflow-manager will not use prometheus.")
var kubeconfigPath = flag.String("kube-config-path", "", "Path to the kubeconfig file to be used to authenticate to Kubernetes API")
var dryRun = flag.Bool("dry-run", false, "If set, no operations with side effects will be done.")
var taskQueueKind = flag.String("task-queue-kind", "", "Which task queue kind to use.")
var intakeTasksTopic = flag.String("intake-tasks-topic", "", "Name of the topic to which intake-batch tasks should be published")
var aggregateTasksTopic = flag.String("aggregate-tasks-topic", "", "Name of the topic to which aggregate tasks should be published")
var maxEnqueueWorkers = flag.Int("max-enqueue-workers", 100, "Max number of workers that can be used to enqueue jobs")

// Arguments for gcp-pubsub task queue
var gcpPubSubCreatePubSubTopics = flag.Bool("gcp-pubsub-create-topics", false, "Whether to create the GCP PubSub topics used for intake and aggregation tasks.")
var gcpProjectID = flag.String("gcp-project-id", "", "Name of the GCP project ID being used for PubSub.")

// Arguments for aws-sns task queue
var awsSNSRegion = flag.String("aws-sns-region", "", "AWS region in which to publish to SNS topic")
var awsSNSIdentity = flag.String("aws-sns-identity", "", "AWS IAM ARN of the role to be assumed to publish to SNS topics")

// Define flags and arguments for other task queue implementations here.
// Argument names should be prefixed with the corresponding value of
// task-queue-kind to avoid conflicts.

// monitoring things
var (
	intakesStarted            monitor.GaugeMonitor = &monitor.NoopGauge{}
	intakesSkippedDueToAge    monitor.GaugeMonitor = &monitor.NoopGauge{}
	intakesSkippedDueToMarker monitor.GaugeMonitor = &monitor.NoopGauge{}

	aggregationsStarted            monitor.GaugeMonitor = &monitor.NoopGauge{}
	aggregationsSkippedDueToMarker monitor.GaugeMonitor = &monitor.NoopGauge{}

	workflowManagerLastSuccess monitor.GaugeMonitor = &monitor.NoopGauge{}
	workflowManagerLastFailure monitor.GaugeMonitor = &monitor.NoopGauge{}
	workflowManagerRuntime     monitor.GaugeMonitor = &monitor.NoopGauge{}
)

func fail(format string, args ...interface{}) {
	workflowManagerLastFailure.SetToCurrentTime()
	log.Fatalf(format, args...)
}

func main() {
	startTime := time.Now()
	log.Printf("starting %s version %s. Args: %s", os.Args[0], BuildInfo, os.Args[1:])
	flag.Parse()

	if *pushGateway != "" {
		pusher := push.New(*pushGateway, "workflow-manager").
			Gatherer(prometheus.DefaultGatherer).
			Grouping("locality", *k8sNS).
			Grouping("ingestor", *ingestorLabel)

		defer pusher.Push()
		intakesStarted = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_intake_tasks_scheduled",
			Help: "The number of intake-batch tasks successfully scheduled",
		})
		intakesSkippedDueToAge = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_intake_tasks_skipped_due_to_age",
			Help: "The number of intake-batch tasks not scheduled because the batch was too old",
		})
		intakesSkippedDueToMarker = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_intake_tasks_skipped_due_to_marker",
			Help: "The number of intake-batch tasks not scheduled because a task marker was found",
		})

		aggregationsStarted = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_aggregation_tasks_scheduled",
			Help: "The number of aggregate tasks successfully scheduled",
		})
		aggregationsSkippedDueToMarker = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_aggregation_tasks_skipped_due_to_marker",
			Help: "The number of aggregate tasks not scheduled because a task marker was found",
		})

		workflowManagerLastSuccess = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_last_success_seconds",
			Help: "Time of last successful run of workflow-manager in seconds since UNIX epoch",
		})

		workflowManagerLastFailure = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_last_failure_seconds",
			Help: "Time of last failed run of workflow-manager in seconds since UNIX epoch",
		})

		workflowManagerRuntime = promauto.NewGauge(prometheus.GaugeOpts{
			Name: "workflow_manager_runtime_seconds",
			Help: "How long successful workflow-manager runs take",
		})
	}

	ownValidationBucket, err := bucket.New(*ownValidationInput, *ownValidationIdentity, *dryRun)
	if err != nil {
		fail("--own-validation-input: %s", err)
		return
	}
	peerValidationBucket, err := bucket.New(*peerValidationInput, *peerValidationIdentity, *dryRun)
	if err != nil {
		fail("--peer-validation-input: %s", err)
		return
	}
	intakeBucket, err := bucket.New(*ingestorInput, *ingestorIdentity, *dryRun)
	if err != nil {
		fail("--ingestor-input: %s", err)
		return
	}

	maxAgeParsed, err := time.ParseDuration(*maxAge)
	if err != nil {
		fail("--max-age: %s", err)
		return
	}

	gracePeriodParsed, err := time.ParseDuration(*gracePeriod)
	if err != nil {
		fail("--grace-period: %s", err)
		return
	}

	aggregationPeriodParsed, err := time.ParseDuration(*aggregationPeriod)
	if err != nil {
		fail("--aggregation-time-slice: %s", err)
		return
	}

	if *taskQueueKind == "" || *intakeTasksTopic == "" || *aggregateTasksTopic == "" {
		fail("--task-queue-kind, --intake-tasks-topic and --aggregate-tasks-topic are required")
		return
	}

	var intakeTaskEnqueuer task.Enqueuer
	var aggregationTaskEnqueuer task.Enqueuer

	switch *taskQueueKind {
	case "gcp-pubsub":
		if *gcpProjectID == "" {
			fail("--gcp-project-id is required for task-queue-kind=gcp-pubsub")
			return
		}

		if *gcpPubSubCreatePubSubTopics {
			if err := task.CreatePubSubTopic(
				*gcpProjectID,
				*intakeTasksTopic,
			); err != nil {
				fail("creating pubsub topic: %s", err)
				return
			}
			if err := task.CreatePubSubTopic(
				*gcpProjectID,
				*aggregateTasksTopic,
			); err != nil {
				fail("creating pubsub topic: %s", err)
				return
			}
		}

		intakeTaskEnqueuer, err = task.NewGCPPubSubEnqueuer(
			*gcpProjectID,
			*intakeTasksTopic,
			*dryRun,
			int32(*maxEnqueueWorkers),
		)
		if err != nil {
			fail("%s", err)
			return
		}

		aggregationTaskEnqueuer, err = task.NewGCPPubSubEnqueuer(
			*gcpProjectID,
			*aggregateTasksTopic,
			*dryRun,
			int32(*maxEnqueueWorkers),
		)
		if err != nil {
			fail("%s", err)
			return
		}
	case "aws-sns":
		if *awsSNSRegion == "" {
			fail("--aws-sns-region is required for task-queue-kind=aws-sns")
			return
		}

		intakeTaskEnqueuer, err = task.NewAWSSNSEnqueuer(
			*awsSNSRegion,
			*awsSNSIdentity,
			*intakeTasksTopic,
			*dryRun,
		)
		if err != nil {
			fail("%s", err)
			return
		}

		aggregationTaskEnqueuer, err = task.NewAWSSNSEnqueuer(
			*awsSNSRegion,
			*awsSNSIdentity,
			*aggregateTasksTopic,
			*dryRun,
		)
		if err != nil {
			fail("%s", err)
			return
		}
	// To implement a new task queue kind, add a case here. You should
	// initialize intakeTaskEnqueuer and aggregationTaskEnqueuer.
	default:
		fail("unknown task queue kind %s", *taskQueueKind)
		return
	}

	intakeFiles, err := intakeBucket.ListFiles()
	if err != nil {
		fail("%s", err)
		return
	}

	ownValidationFiles, err := ownValidationBucket.ListFiles()
	if err != nil {
		fail("%s", err)
		return
	}

	peerValidationFiles, err := peerValidationBucket.ListFiles()
	if err != nil {
		fail("%s", err)
		return
	}

	scheduleTasks(scheduleTasksConfig{
		isFirst:                 *isFirst,
		clock:                   utils.DefaultClock(),
		intakeFiles:             intakeFiles,
		ownValidationFiles:      ownValidationFiles,
		peerValidationFiles:     peerValidationFiles,
		intakeTaskEnqueuer:      intakeTaskEnqueuer,
		aggregationTaskEnqueuer: aggregationTaskEnqueuer,
		ownValidationBucket:     ownValidationBucket,
		maxAge:                  maxAgeParsed,
		aggregationPeriod:       aggregationPeriodParsed,
		gracePeriod:             gracePeriodParsed,
	})

	workflowManagerLastSuccess.SetToCurrentTime()
	endTime := time.Now()

	workflowManagerRuntime.Set(endTime.Sub(startTime).Seconds())

	log.Print("done")
}

type scheduleTasksConfig struct {
	isFirst                                              bool
	clock                                                utils.Clock
	intakeFiles, ownValidationFiles, peerValidationFiles []string
	intakeTaskEnqueuer, aggregationTaskEnqueuer          task.Enqueuer
	ownValidationBucket                                  bucket.TaskMarkerWriter
	maxAge, aggregationPeriod, gracePeriod               time.Duration
}

// scheduleTasks evaluates bucket contents and Kubernetes cluster state to
// schedule new tasks
func scheduleTasks(config scheduleTasksConfig) error {
	intakeBatches, err := batchpath.ReadyBatches(config.intakeFiles, "batch")
	if err != nil {
		return err
	}

	// Make a set of the tasks for which we have marker objects for efficient
	// lookup later.
	taskMarkers := map[string]struct{}{}
	for _, object := range config.ownValidationFiles {
		if !strings.HasPrefix(object, "task-markers/") {
			continue
		}
		taskMarkers[strings.TrimPrefix(object, "task-markers/")] = struct{}{}
	}

	currentIntakeBatches := withinInterval(intakeBatches, interval{
		begin: config.clock.Now().Add(-config.maxAge),
		end:   config.clock.Now().Add(24 * time.Hour),
	})
	log.Printf("skipping %d batches as too old", len(intakeBatches)-len(currentIntakeBatches))

	err = enqueueIntakeTasks(
		config.clock,
		currentIntakeBatches,
		config.maxAge,
		taskMarkers,
		config.ownValidationBucket,
		config.intakeTaskEnqueuer,
	)
	if err != nil {
		return err
	}

	ownValidityInfix := fmt.Sprintf("validity_%d", utils.Index(config.isFirst))
	ownValidationBatches, err := batchpath.ReadyBatches(config.ownValidationFiles, ownValidityInfix)
	if err != nil {
		return err
	}

	log.Printf("found %d own validations", len(ownValidationBatches))

	peerValidityInfix := fmt.Sprintf("validity_%d", utils.Index(!config.isFirst))
	peerValidationBatches, err := batchpath.ReadyBatches(config.peerValidationFiles, peerValidityInfix)
	if err != nil {
		return err
	}

	log.Printf("found %d peer validations", len(peerValidationBatches))

	// Take the intersection of the sets of own validations and peer validations
	// to get the list of batches we can aggregate.
	// Go doesn't have sets, so we have to use a map[string]bool. We use the
	// batch ID as the key to the set, because batchPath is not a valid map key
	// type, and using a *batchPath wouldn't give us the lookup semantics we
	// want.
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

	interval := aggregationInterval(config.clock, config.aggregationPeriod, config.gracePeriod)
	log.Printf("looking for batches to aggregate in interval %s", interval)
	aggregationBatches = withinInterval(aggregationBatches, interval)
	aggregationMap := groupByAggregationID(aggregationBatches)
	err = enqueueAggregationTasks(
		aggregationMap,
		interval,
		taskMarkers,
		config.ownValidationBucket,
		config.aggregationTaskEnqueuer,
	)
	if err != nil {
		return err
	}

	// Ensure both task enqueuers have completed their asynchronous work before
	// allowing the process to exit
	config.intakeTaskEnqueuer.Stop()
	config.aggregationTaskEnqueuer.Stop()

	return nil
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

// aggregationInterval calculates the interval we want to run an aggregation for, if any.
// That is whatever interval is `gracePeriod` earlier than now and aligned on multiples
// of `aggregationPeriod` (relative to the zero time).
func aggregationInterval(clock utils.Clock, aggregationPeriod, gracePeriod time.Duration) interval {
	var output interval
	output.end = clock.Now().Add(-gracePeriod).Truncate(aggregationPeriod)
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

func enqueueAggregationTasks(
	batchesByID aggregationMap,
	inter interval,
	taskMarkers map[string]struct{},
	ownValidationBucket bucket.TaskMarkerWriter,
	enqueuer task.Enqueuer,
) error {
	if len(batchesByID) == 0 {
		log.Printf("no batches to aggregate")
		return nil
	}

	skippedDueToMarker := 0
	scheduled := 0

	for _, readyBatches := range batchesByID {
		aggregationID := readyBatches[0].AggregationID
		batches := []task.Batch{}

		batchCount := 0
		for _, batchPath := range readyBatches {
			batchCount++
			batches = append(batches, task.Batch{
				ID:   batchPath.ID,
				Time: task.Timestamp(batchPath.Time),
			})

			// All batches should have the same aggregation ID?
			if aggregationID != batchPath.AggregationID {
				return fmt.Errorf("found batch with aggregation ID %s, wanted %s", batchPath.AggregationID, aggregationID)
			}
		}

		aggregationTask := task.Aggregation{
			AggregationID:    aggregationID,
			AggregationStart: task.Timestamp(inter.begin),
			AggregationEnd:   task.Timestamp(inter.end),
			Batches:          batches,
		}

		if _, ok := taskMarkers[aggregationTask.Marker()]; ok {
			skippedDueToMarker++
			aggregationsSkippedDueToMarker.Inc()
			continue
		}

		log.Printf("scheduling aggregation task over interval %s for aggregation ID %s over %d batches",
			inter, aggregationID, batchCount)
		scheduled++
		enqueuer.Enqueue(aggregationTask, func(err error) {
			if err != nil {
				log.Printf("failed to enqueue aggregation task: %s", err)
				return
			}

			// Write a marker to cloud storage to ensure we don't schedule
			// redundant tasks
			if err := ownValidationBucket.WriteTaskMarker(aggregationTask.Marker()); err != nil {
				log.Printf("failed to write aggregation task marker: %s", err)
			}

			aggregationsStarted.Inc()
		})
	}

	log.Printf("skipped %d aggregation tasks due to markers. Scheduled %d new aggregation tasks.",
		skippedDueToMarker, scheduled)

	return nil
}

func enqueueIntakeTasks(
	clock utils.Clock,
	readyBatches batchpath.List,
	ageLimit time.Duration,
	taskMarkers map[string]struct{},
	ownValidationBucket bucket.TaskMarkerWriter,
	enqueuer task.Enqueuer,
) error {
	skippedDueToAge := 0
	skippedDueToMarker := 0
	scheduled := 0

	for _, batch := range readyBatches {
		age := clock.Now().Sub(batch.Time)
		if age > ageLimit {
			skippedDueToAge++
			intakesSkippedDueToAge.Inc()
			continue
		}

		intakeTask := task.IntakeBatch{
			AggregationID: batch.AggregationID,
			BatchID:       batch.ID,
			Date:          task.Timestamp(batch.Time),
		}

		if _, ok := taskMarkers[intakeTask.Marker()]; ok {
			skippedDueToMarker++
			intakesSkippedDueToMarker.Inc()
			continue
		}

		log.Printf("scheduling intake task for batch %s", batch)
		scheduled++
		enqueuer.Enqueue(intakeTask, func(err error) {
			if err != nil {
				log.Printf("failed to enqueue intake task: %s", err)
				return
			}
			// Write a marker to cloud storage to ensure we don't schedule
			// redundant tasks
			if err := ownValidationBucket.WriteTaskMarker(intakeTask.Marker()); err != nil {
				log.Printf("failed to write intake task marker: %s", err)
				return
			}

			intakesStarted.Inc()
		})
	}

	log.Printf("skipped %d batches as too old, %d with existing tasks. Scheduled %d new intake tasks.",
		skippedDueToAge, skippedDueToMarker, scheduled)

	return nil
}
