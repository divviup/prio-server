// task-replayer is a simple utility for backfills. It allows operators to
// replay prio-server tasks into a task queue.
package main

import (
	"encoding/json"
	"flag"
	"log"
	"os"

	"github.com/letsencrypt/prio-server/workflow-manager/task"
)

var taskQueueKind = flag.String(
	"task-queue-kind",
	"",
	`Which task queue kind to use. Can be either "gcp-pubsub" or "aws-sns".`,
)
var gcpProjectID = flag.String(
	"gcp-project-id",
	"",
	"Name of the GCP project ID being used for PubSub. Required if --task-queue-kind=gcp-pubsub.",
)
var awsSNSRegion = flag.String(
	"aws-sns-region",
	"",
	"AWS region in which to publish to SNS topic. Required if --task-queue-kind=aws-sns.",
)
var awsSNSIdentity = flag.String(
	"aws-sns-identity",
	"",
	"AWS IAM ARN of the role to be assumed to publish to SNS topics. Required if --task-queue-kind=aws-sns",
)
var topic = flag.String("topic", "", "Name of the topic to which replayed tasks should be published")
var intakeBatchTaskFile = flag.String(
	"intake-batch-task",
	"",
	`File containing a single JSON encoded intake-batch task to be replayed. If specified,
task-replayer will replay the single task and exit. May not be specified alongside
--aggregate-task.`,
)
var aggregateTaskFile = flag.String(
	"aggregate-task",
	"",
	`File containing a single JSON encoded aggregate task to be replayed. If specified,
task-replayer will replay the single task and exit. May not be specified alongside
--intake-batch-task.`,
)
var dryRun = flag.Bool("dry-run", false, "If set, no operations with side effects will be done.")

func main() {
	flag.Parse()

	if *taskQueueKind == "" {
		log.Fatal("--task-queue-kind is required")
	}

	var taskEnqueuer task.Enqueuer
	var err error

	if *topic == "" {
		log.Fatal("--topic is required")
	}

	switch *taskQueueKind {
	case "gcp-pubsub":
		if *gcpProjectID == "" {
			log.Fatal("--gcp-project-id is required for task-queue-kind=gcp-pubsub")
		}
		taskEnqueuer, err = task.NewGCPPubSubEnqueuer(*gcpProjectID, *topic, *dryRun, 10)
		if err != nil {
			log.Fatalf("%s", err)
		}
	case "aws-sns":
		if *awsSNSRegion == "" {
			log.Fatalf("--aws-sns-region is required for task-queue-kind=aws-sns")
		}
		taskEnqueuer, err = task.NewAWSSNSEnqueuer(*awsSNSRegion, *awsSNSIdentity, *topic, *dryRun)
		if err != nil {
			log.Fatalf("%s", err)
		}
	default:
		log.Fatalf("unknown task queue kind %s", *taskQueueKind)
	}

	var taskToReplay task.Task
	if *intakeBatchTaskFile != "" {
		file, err := os.Open(*intakeBatchTaskFile)
		if err != nil {
			log.Fatalf("failed to open task file %s: %s", *intakeBatchTaskFile, err)
		}
		var intakeBatchTask task.IntakeBatch
		if err := json.NewDecoder(file).Decode(&intakeBatchTask); err != nil {
			log.Fatalf("failed to decode intake batch task from file %s: %s", *intakeBatchTaskFile, err)
		}

		taskToReplay = intakeBatchTask
	} else if *aggregateTaskFile != "" {
		file, err := os.Open(*aggregateTaskFile)
		if err != nil {
			log.Fatalf("failed to open task file %s: %s", *aggregateTaskFile, err)
		}
		var aggregateTask task.Aggregation
		if err := json.NewDecoder(file).Decode(&aggregateTask); err != nil {
			log.Fatalf("failed to decode aggregate task from file %s: %s", *aggregateTaskFile, err)
		}

		taskToReplay = aggregateTask
	} else {
		log.Fatalf("one of --intake-batch-task or --aggregate-task must be specified")
	}

	taskEnqueuer.Enqueue(taskToReplay, func(err error) {
		if err != nil {
			log.Fatalf("failed to enqueue task: %s", err)
		}
	})

	taskEnqueuer.Stop()
}
