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
	"os"
	"strings"
	"time"

	"github.com/rs/zerolog"
	"github.com/rs/zerolog/log"

	"github.com/letsencrypt/prio-server/workflow-manager/batchpath"
	"github.com/letsencrypt/prio-server/workflow-manager/storage"
	"github.com/letsencrypt/prio-server/workflow-manager/task"
	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"
	"github.com/letsencrypt/prio-server/workflow-manager/utils"

	"github.com/google/uuid"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"github.com/prometheus/client_golang/prometheus/push"
)

// BuildInfo is generated at build time - see the Dockerfile.
var BuildInfo string

// Flags.
var (
	k8sNS                  = flag.String("k8s-namespace", "", "Kubernetes namespace")
	ingestorLabel          = flag.String("ingestor-label", "", "Label of ingestion server")
	isFirst                = flag.Bool("is-first", false, "Whether this set of servers is \"first\", aka PHA servers")
	maxAge                 = flag.Duration("intake-max-age", time.Hour, "Max age (in Go duration format) for intake batches to be worth processing.")
	ingestorInput          = flag.String("ingestor-input", "", "Bucket for input from ingestor (s3:// or gs://) (Required)")
	ingestorIdentity       = flag.String("ingestor-identity", "", "Identity to use with ingestor bucket (Required for S3)")
	ownValidationInput     = flag.String("own-validation-input", "", "Bucket for input of validation batches from self (s3:// or gs://) (required)")
	ownValidationIdentity  = flag.String("own-validation-identity", "", "Identity to use with own validation bucket (Required for S3)")
	peerValidationInput    = flag.String("peer-validation-input", "", "Bucket for input of validation batches from peer (s3:// or gs://) (required)")
	peerValidationIdentity = flag.String("peer-validation-identity", "", "Identity to use with peer validation bucket (Required for S3)")
	pushGateway            = flag.String("push-gateway", "", "Set this to the gateway to use with prometheus. If left empty, workflow-manager will not use prometheus.")
	dryRun                 = flag.Bool("dry-run", false, "If set, no operations with side effects will be done.")
	taskQueueKind          = flag.String("task-queue-kind", "", "Which task queue kind to use.")
	intakeTasksTopic       = flag.String("intake-tasks-topic", "", "Name of the topic to which intake-batch tasks should be published")
	aggregateTasksTopic    = flag.String("aggregate-tasks-topic", "", "Name of the topic to which aggregate tasks should be published")
	maxEnqueueWorkers      = flag.Int("max-enqueue-workers", 100, "Max number of workers that can be used to enqueue jobs")

	// Aggregation window flags, which determine which aggregation window will be aggregated (if not already aggregated).
	// Normally, aggregation occurs for the window including the point in time of (now - grace-period).
	// If aggregation-override-timestamp is specified, the aggregation window containing the override point will be aggregated instead.
	aggregationPeriod            = flag.Duration("aggregation-period", 3*time.Hour, "How much time each aggregation covers")
	gracePeriod                  = flag.Duration("grace-period", time.Hour, "Wait this amount of time after the end of an aggregation timeslice to run the aggregation. Relevant only if --aggregation-override-point is unset")
	aggregationOverrideTimestamp = flag.String("aggregation-override-timestamp", "", "If specified, a point inside the aggregation window to be aggregated, in the format YYYYMMDDHHmm")

	// Arguments for gcp-pubsub task queue
	gcpPubSubCreatePubSubTopics = flag.Bool("gcp-pubsub-create-topics", false, "Whether to create the GCP PubSub topics used for intake and aggregation tasks.")
	gcpProjectID                = flag.String("gcp-project-id", "", "Name of the GCP project ID being used for PubSub.")

	// Arguments for aws-sns task queue
	awsSNSRegion   = flag.String("aws-sns-region", "", "AWS region in which to publish to SNS topic")
	awsSNSIdentity = flag.String("aws-sns-identity", "", "AWS IAM ARN of the role to be assumed to publish to SNS topics")

	// Define flags and arguments for other task queue implementations here.
	// Argument names should be prefixed with the corresponding value of
	// task-queue-kind to avoid conflicts.
)

// Metrics gauges. We must use gauges because workflow-manager runs as a
// cronjob, and so if we used counters, they would be reset to zero with each
// run.
var (
	ingestionBatchesFound = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_ingestions_found",
			Help: "The number of ingestion batches found in the current aggregation interval",
		},
		[]string{"aggregation_id"},
	)
	incompleteIngestionBatchesFound = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_incomplete_ingestions_found",
			Help: "The number of incomplete ingestion batches found in the current aggregation interval",
		},
		[]string{"aggregation_id"},
	)

	ownValidationsFound = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_own_validations_found",
			Help: "The number of own validation batches found in the current aggregation interval",
		},
		[]string{"aggregation_id"},
	)
	incompleteOwnValidationsFound = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_incomplete_own_validations_found",
			Help: "The number of incomplete own validation batches found in the current aggregation interval",
		},
		[]string{"aggregation_id"},
	)

	peerValidationsFound = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_peer_validations_found",
			Help: "The number of peer validation batches found in the current aggregation interval",
		},
		[]string{"aggregation_id"},
	)
	incompletePeerValidationsFound = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_incomplete_peer_validations_found",
			Help: "The number of incomplete peer validation batches found in the current aggregation interval",
		},
		[]string{"aggregation_id"},
	)

	intakesStarted = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_intake_tasks_scheduled",
			Help: "The number of intake-batch tasks successfully scheduled",
		},
		[]string{"aggregation_id"},
	)
	intakesSkippedDueToMarker = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_intake_tasks_skipped_due_to_marker",
			Help: "The number of intake-batch tasks not scheduled because a task marker was found",
		},
		[]string{"aggregation_id"},
	)

	aggregationsStarted = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_aggregation_tasks_scheduled",
			Help: "The number of aggregate tasks successfully scheduled",
		},
		[]string{"aggregation_id"},
	)
	aggregationsSkippedDueToMarker = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_aggregation_tasks_skipped_due_to_marker",
			Help: "The number of aggregate tasks not scheduled because a task marker was found",
		},
		[]string{"aggregation_id"},
	)
	numberOfBatchesInAggregation = promauto.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "workflow_manager_number_of_batches_in_aggregation",
			Help: "The number of batches included in a scheduled aggregation",
		},
		[]string{"aggregation_id"},
	)

	// Unlike all the other metrics, these are not specific to a particular
	// aggregation, and so do not have the `aggregation_id` label, and so we do
	// not need a Gauge*Vec*.
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
)

func fail(format string, args ...interface{}) {
	workflowManagerLastFailure.SetToCurrentTime()
	log.Fatal().Msgf(format, args...)
}

func prepareLogger() {
	zerolog.LevelFieldName = "severity"
	zerolog.TimestampFieldName = "timestamp"
	zerolog.TimeFieldFormat = time.RFC3339Nano
}

func main() {
	prepareLogger()
	startTime := time.Now()
	log.Info().
		Str("app", os.Args[0]).
		Str("version", BuildInfo).
		Str("Args", strings.Join(os.Args[1:], ",")).
		Msgf("starting %s version %s. Args: %s", os.Args[0], BuildInfo, os.Args[1:])
	flag.Parse()

	if *pushGateway != "" {
		pusher := push.New(*pushGateway, "workflow-manager").
			Gatherer(prometheus.DefaultGatherer).
			Grouping("locality", *k8sNS).
			Grouping("ingestor", *ingestorLabel)
		defer func() {
			err := pusher.Push()
			if err != nil {
				log.Err(err).Msg("error occurred with pushing to prometheus")
			}
		}()
	}

	ownValidationBucket, err := storage.NewBucket(*ownValidationInput, *ownValidationIdentity, *dryRun)
	if err != nil {
		fail("--own-validation-input: %s", err)
		return
	}
	peerValidationBucket, err := storage.NewBucket(*peerValidationInput, *peerValidationIdentity, *dryRun)
	if err != nil {
		fail("--peer-validation-input: %s", err)
		return
	}
	intakeBucket, err := storage.NewBucket(*ingestorInput, *ingestorIdentity, *dryRun)
	if err != nil {
		fail("--ingestor-input: %s", err)
		return
	}

	var aggregationInterval aggregationIntervalFunc
	if *aggregationOverrideTimestamp == "" {
		aggregationInterval = standardAggregationWindow(*aggregationPeriod, *gracePeriod)
	} else {
		const timeLayout = "200601021504" // YYYYMMDDHHmm, e.g. 202110041600
		when, err := time.Parse(timeLayout, *aggregationOverrideTimestamp)
		if err != nil {
			fail("--aggregation-override-timestamp: couldn't parse %q as time: %v", *aggregationOverrideTimestamp, err)
		}
		aggregationInterval = overrideAggregationWindow(when, *aggregationPeriod)
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

	aggregationIDs, err := intakeBucket.ListAggregationIDs()
	if err != nil {
		fail("unable to discover aggregation IDs from ingestion bucket: %q", err)
		return
	}

	for _, aggregationID := range aggregationIDs {
		err = scheduleTasks(scheduleTasksConfig{
			aggregationID:           aggregationID,
			isFirst:                 *isFirst,
			clock:                   wftime.DefaultClock(),
			intakeBucket:            intakeBucket,
			ownValidationBucket:     ownValidationBucket,
			peerValidationBucket:    peerValidationBucket,
			intakeTaskEnqueuer:      intakeTaskEnqueuer,
			aggregationTaskEnqueuer: aggregationTaskEnqueuer,
			maxAge:                  *maxAge,
			aggregationInterval:     aggregationInterval,
		})

		if err != nil {
			log.Err(err).Str("aggregation ID", aggregationID).Msgf("Failed to schedule aggregation tasks: %s", err)
			return
		}
	}

	workflowManagerLastSuccess.SetToCurrentTime()

	endTime := time.Now()
	workflowManagerRuntime.Set(endTime.Sub(startTime).Seconds())

	log.Info().Msg("done")
}

type scheduleTasksConfig struct {
	aggregationID                                           string
	isFirst                                                 bool
	clock                                                   wftime.Clock
	intakeBucket, ownValidationBucket, peerValidationBucket storage.Bucket
	intakeTaskEnqueuer, aggregationTaskEnqueuer             task.Enqueuer
	maxAge                                                  time.Duration
	aggregationInterval                                     aggregationIntervalFunc
}

// scheduleTasks evaluates bucket contents and Kubernetes cluster state to
// schedule new tasks
func scheduleTasks(config scheduleTasksConfig) error {
	intakeInterval := wftime.Interval{
		Begin: config.clock.Now().Add(-config.maxAge),
		End:   config.clock.Now().Add(24 * time.Hour),
	}

	intakeFiles, err := config.intakeBucket.ListBatchFiles(config.aggregationID, intakeInterval)
	if err != nil {
		return err
	}

	intakeBatches, err := batchpath.ReadyBatches(intakeFiles, "batch")
	if err != nil {
		return err
	}

	ingestionBatchesFound.WithLabelValues(config.aggregationID).Set(float64(intakeBatches.Batches.Len()))
	incompleteIngestionBatchesFound.WithLabelValues(config.aggregationID).Set(float64(intakeBatches.IncompleteBatchCount))
	log.Info().
		Int("ingestion batches", intakeBatches.Batches.Len()).
		Int("incomplete ingestion batches", intakeBatches.IncompleteBatchCount).
		Msg("discovered ingestion batches")

	// Make a set of the tasks for which we have marker objects for efficient
	// lookup later.
	intakeTaskMarkers, err := config.ownValidationBucket.ListIntakeTaskMarkers(config.aggregationID, intakeInterval)
	if err != nil {
		return err
	}

	intakeTaskMarkersSet := map[string]struct{}{}
	for _, marker := range intakeTaskMarkers {
		intakeTaskMarkersSet[marker] = struct{}{}
	}

	err = enqueueIntakeTasks(
		intakeBatches.Batches,
		intakeTaskMarkersSet,
		config.ownValidationBucket,
		config.intakeTaskEnqueuer,
	)
	if err != nil {
		return err
	}

	aggInterval := config.aggregationInterval(config.clock.Now())

	log.Info().Str("aggregation interval", aggInterval.String()).Msgf("looking for batches to aggregate in interval %s", aggInterval)

	ownValidationFiles, err := config.ownValidationBucket.ListBatchFiles(config.aggregationID, aggInterval)
	if err != nil {
		return err
	}

	ownValidityInfix := fmt.Sprintf("validity_%d", utils.Index(config.isFirst))
	ownValidationBatches, err := batchpath.ReadyBatches(ownValidationFiles, ownValidityInfix)
	if err != nil {
		return err
	}

	ownValidationsFound.WithLabelValues(config.aggregationID).Set(float64(ownValidationBatches.Batches.Len()))
	incompleteOwnValidationsFound.WithLabelValues(config.aggregationID).Set(float64(ownValidationBatches.IncompleteBatchCount))
	log.Info().
		Int("own validations", ownValidationBatches.Batches.Len()).
		Int("incomplete own validations", ownValidationBatches.IncompleteBatchCount).
		Msg("discovered own validations")

	peerValidationFiles, err := config.peerValidationBucket.ListBatchFiles(config.aggregationID, aggInterval)
	if err != nil {
		return err
	}

	peerValidityInfix := fmt.Sprintf("validity_%d", utils.Index(!config.isFirst))
	peerValidationBatches, err := batchpath.ReadyBatches(peerValidationFiles, peerValidityInfix)
	if err != nil {
		return err
	}

	peerValidationsFound.WithLabelValues(config.aggregationID).Set(float64(peerValidationBatches.Batches.Len()))
	incompletePeerValidationsFound.WithLabelValues(config.aggregationID).Set(float64(peerValidationBatches.IncompleteBatchCount))
	log.Info().
		Int("peer validations", peerValidationBatches.Batches.Len()).
		Int("incomplete peer validations", peerValidationBatches.IncompleteBatchCount).
		Msg("discovered peer validations")

	// Take the intersection of the sets of own validations and peer validations
	// to get the list of batches we can aggregate.
	// Go doesn't have sets, so we have to use a map[string]bool. We use the
	// batch ID as the key to the set, because batchPath is not a valid map key
	// type, and using a *batchPath wouldn't give us the lookup semantics we
	// want.
	ownValidationsSet := map[string]bool{}
	for _, ownValidationBatch := range ownValidationBatches.Batches {
		ownValidationsSet[ownValidationBatch.ID] = true
	}
	aggregationBatches := batchpath.List{}
	for _, peerValidationBatch := range peerValidationBatches.Batches {
		if _, ok := ownValidationsSet[peerValidationBatch.ID]; ok {
			aggregationBatches = append(aggregationBatches, peerValidationBatch)
		}
	}

	aggregationTaskMarkers, err := config.ownValidationBucket.ListAggregateTaskMarkers(config.aggregationID)
	if err != nil {
		return err
	}
	aggregationTaskMarkersSet := map[string]struct{}{}
	for _, marker := range aggregationTaskMarkers {
		aggregationTaskMarkersSet[marker] = struct{}{}
	}

	err = enqueueAggregationTask(
		config.aggregationID,
		aggregationBatches,
		aggInterval,
		aggregationTaskMarkersSet,
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

func enqueueAggregationTask(
	aggregationID string,
	readyBatches batchpath.List,
	aggregationWindow wftime.Interval,
	taskMarkers map[string]struct{},
	ownValidationBucket storage.Bucket,
	enqueuer task.Enqueuer,
) error {
	if len(readyBatches) == 0 {
		log.Info().Msg("no batches to aggregate")
		return nil
	}

	batches := []task.Batch{}

	batchCount := 0
	for _, batchPath := range readyBatches {
		batchCount++
		batches = append(batches, task.Batch{
			ID:   batchPath.ID,
			Time: wftime.Timestamp(batchPath.Time),
		})

		// All batches should have the same aggregation ID?
		if aggregationID != batchPath.AggregationID {
			return fmt.Errorf("found batch with aggregation ID %s, wanted %s", batchPath.AggregationID, aggregationID)
		}
	}

	aggregationTask := task.Aggregation{
		TraceID:          uuid.New(),
		AggregationID:    aggregationID,
		AggregationStart: wftime.Timestamp(aggregationWindow.Begin),
		AggregationEnd:   wftime.Timestamp(aggregationWindow.End),
		Batches:          batches,
	}

	if _, ok := taskMarkers[aggregationTask.Marker()]; ok {
		aggregationTask.PrepareLog(log.Info()).
			Msg("skipped aggregation task due to marker")
		aggregationsSkippedDueToMarker.WithLabelValues(aggregationID).Inc()
		return nil
	}

	aggregationTask.PrepareLog(log.Info()).
		Str("aggregation window", aggregationWindow.String()).
		Msg("Scheduling aggregation task")

	enqueuer.Enqueue(aggregationTask, func(err error) {
		if err != nil {
			aggregationTask.PrepareLog(log.Err(err)).
				Msgf("failed to enqueue aggregation task: %s", err)
			return
		}

		// Write a marker to cloud storage to ensure we don't schedule redundant
		// tasks
		if err := ownValidationBucket.WriteTaskMarker(aggregationTask.Marker()); err != nil {
			aggregationTask.PrepareLog(log.Err(err)).
				Msgf("failed to write aggregation task marker: %s", err)
		}

		aggregationsStarted.WithLabelValues(aggregationID).Inc()
		numberOfBatchesInAggregation.WithLabelValues(aggregationID).Set(float64(batchCount))
	})

	return nil
}

func enqueueIntakeTasks(
	readyBatches batchpath.List,
	taskMarkers map[string]struct{},
	ownValidationBucket storage.Bucket,
	enqueuer task.Enqueuer,
) error {
	skippedDueToMarker := 0
	scheduled := 0

	for _, batch := range readyBatches {
		intakeTask := task.IntakeBatch{
			AggregationID: batch.AggregationID,
			BatchID:       batch.ID,
			Date:          wftime.Timestamp(batch.Time),
			TraceID:       uuid.New(),
		}

		if _, ok := taskMarkers[intakeTask.Marker()]; ok {
			skippedDueToMarker++
			intakesSkippedDueToMarker.WithLabelValues(batch.AggregationID).Inc()
			continue
		}

		intakeTask.PrepareLog(log.Info()).
			Str("batch", batch.String()).
			Msg("scheduling intake task for batch")

		scheduled++
		enqueuer.Enqueue(intakeTask, func(err error) {
			if err != nil {
				intakeTask.PrepareLog(log.Err(err)).
					Msg("failed to enqueue intake task")
				return
			}
			// Write a marker to cloud storage to ensure we don't schedule
			// redundant tasks
			if err := ownValidationBucket.WriteTaskMarker(intakeTask.Marker()); err != nil {
				intakeTask.PrepareLog(log.Err(err)).
					Msg("failed to write intake task marker")
				return
			}

			intakesStarted.WithLabelValues(batch.AggregationID).Inc()
		})
	}

	log.Info().
		Int("skipped batches", skippedDueToMarker).
		Int("scheduled batches", scheduled).
		Msg("skipped and scheduled intake tasks")

	return nil
}

type aggregationIntervalFunc func(now time.Time) wftime.Interval

func standardAggregationWindow(aggregationPeriod, gracePeriod time.Duration) aggregationIntervalFunc {
	return func(now time.Time) wftime.Interval {
		return wftime.AggregationInterval(now, aggregationPeriod, gracePeriod)
	}
}

func overrideAggregationWindow(when time.Time, aggregationPeriod time.Duration) aggregationIntervalFunc {
	return func(time.Time) wftime.Interval {
		return wftime.AggregationIntervalIncluding(when, aggregationPeriod)
	}
}
