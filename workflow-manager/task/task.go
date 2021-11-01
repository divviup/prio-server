// package task contains representations of tasks that are sent to facilitator
// workers by workflow-manager. A _task_ is a work item at the data share
// processor application layer (i.e., intake of a batch or aggregation of many
// batches); its name is chosen to distinguish from Kubernetes-level _jobs_.
package task

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"github.com/rs/zerolog/log"

	leaws "github.com/letsencrypt/prio-server/workflow-manager/aws"
	"github.com/letsencrypt/prio-server/workflow-manager/limiter"
	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"

	"cloud.google.com/go/pubsub"
	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/service/sns"
)

// Task is a task that can be enqueued into an Enqueuer
type Task interface {
	// Marker returns the name that should be used when writing out a marker for
	// this task
	Marker() string
}

// Aggregation represents an aggregation task
type Aggregation struct {
	// TraceID is the tracing identifier for the aggregation.
	TraceID uuid.UUID `json:"trace-id"`
	// AggregationID is the identifier for the aggregation
	AggregationID string `json:"aggregation-id"`
	// AggregationStart is the start of the range of time covered by the
	// aggregation
	AggregationStart wftime.Timestamp `json:"aggregation-start"`
	// AggregationEnd is the end of the range of time covered by the aggregation
	AggregationEnd wftime.Timestamp `json:"aggregation-end"`
	// Batches is the list of batch ID date pairs of the batches aggregated by
	// this task
	Batches []Batch `json:"batches"`
}

func (a Aggregation) PrepareLog(event *zerolog.Event) *zerolog.Event {
	return event.
		Str("trace ID", a.TraceID.String()).
		Str("aggregation ID", a.AggregationID).
		Int("batch count", len(a.Batches))
}

func (a Aggregation) Marker() string {
	return fmt.Sprintf(
		"aggregate-%s-%s-%s",
		a.AggregationID,
		a.AggregationStart.MarkerString(),
		a.AggregationEnd.MarkerString(),
	)
}

// Batch represents a batch included in an aggregation task
type Batch struct {
	// ID is the batch ID. Typically a UUID.
	ID string `json:"id"`
	// Time is the timestamp on the batch
	Time wftime.Timestamp `json:"time"`
}

type IntakeBatch struct {
	// TraceID is the tracing identifier for the intake batch.
	TraceID uuid.UUID `json:"trace-id"`
	// AggregationID is the identifier for the aggregation
	AggregationID string `json:"aggregation-id"`
	// BatchID is the identifier of the batch. Typically a UUID.
	BatchID string `json:"batch-id"`
	// Date is the timestamp on the batch
	Date wftime.Timestamp `json:"date"`
}

func (i IntakeBatch) PrepareLog(event *zerolog.Event) *zerolog.Event {
	return event.
		Str("trace ID", i.TraceID.String()).
		Str("aggregation ID", i.AggregationID).
		Str("batch ID", i.BatchID)
}

func (i IntakeBatch) Marker() string {
	return fmt.Sprintf("intake-%s-%s-%s", i.AggregationID, i.Date.MarkerString(), i.BatchID)
}

// Enqueuer allows enqueuing tasks.
type Enqueuer interface {
	// Enqueue enqueues a task to be executed later. The provided completion
	// function will be invoked once the task is either successfully enqueued or
	// some unretryable error has occurred. A call to Stop() will not return
	// until completion functions passed to any and all calls to Enqueue() have
	// returned.
	Enqueue(task Task, completion func(error))
	// Stop blocks until all tasks passed to Enqueue() have been enqueued in the
	// underlying system, and all completion functions passed to Enqueue() have
	// returned, and so it is safe to exit the program without losing any tasks.
	Stop()
}

// CreatePubSubTopic creates a PubSub topic with the provided ID, as well as a
// subscription with the same ID that can later be used by a facilitator.
// Returns error on failure.
func CreatePubSubTopic(project string, topicID string) error {
	// Google documentation advises against timeouts on client creation
	// https://godoc.org/cloud.google.com/go#hdr-Timeouts_and_Cancellation
	ctx := context.Background()

	client, err := pubsub.NewClient(ctx, project)
	if err != nil {
		return fmt.Errorf("pubsub.newClient: %w", err)
	}

	topic, err := client.CreateTopic(ctx, topicID)
	if err != nil {
		return fmt.Errorf("pubsub.CreateTopic: %w", err)
	}

	tenMinutes, _ := time.ParseDuration("10m")

	subscriptionConfig := pubsub.SubscriptionConfig{
		Topic:            topic,
		AckDeadline:      tenMinutes,
		ExpirationPolicy: time.Duration(0), // never expire
	}
	if _, err := client.CreateSubscription(ctx, topicID, subscriptionConfig); err != nil {
		return fmt.Errorf("pubsub.CreateSubscription: %w", err)
	}

	return nil
}

// GCPPubSubEnqueuer implements Enqueuer using GCP PubSub
type GCPPubSubEnqueuer struct {
	topic     *pubsub.Topic
	waitGroup sync.WaitGroup
	dryRun    bool
	limiter   *limiter.Limiter
}

// NewGCPPubSubEnqueuer creates a task enqueuer for a given project and topic
// in GCP PubSub. If dryRun is true, no tasks will actually be enqueued. Clients
// should re-use a single instance as much as possible to enable batching of
// publish requests.
func NewGCPPubSubEnqueuer(project string, topicID string, dryRun bool, maxWorkers int32) (*GCPPubSubEnqueuer, error) {
	// Google documentation advises against timeouts on client creation
	// https://godoc.org/cloud.google.com/go#hdr-Timeouts_and_Cancellation
	ctx := context.Background()

	client, err := pubsub.NewClient(ctx, project)
	if err != nil {
		return nil, fmt.Errorf("pubsub.NewClient: %w", err)
	}

	return &GCPPubSubEnqueuer{
		topic:   client.Topic(topicID),
		dryRun:  dryRun,
		limiter: limiter.New(maxWorkers),
	}, nil
}

func (e *GCPPubSubEnqueuer) Enqueue(task Task, completion func(error)) {
	e.limiter.Execute(func(ticket *limiter.Ticket) {
		e.waitGroup.Add(1)
		go func() {
			defer e.waitGroup.Done()
			defer e.limiter.Done(ticket)
			jsonTask, err := json.Marshal(task)
			if err != nil {
				completion(fmt.Errorf("marshaling task to JSON: %w", err))
				return
			}

			if e.dryRun {
				log.Info().Msg("dry run, not enqueuing task")
				completion(nil)
				return
			}

			// Publish() returns immediately, giving us a handle to the result that we
			// can block on to see if publishing succeeded. The PubSub client
			// automatically retries for us, so we just keep the handle so the caller
			// can do whatever they need to after successful publication and we can
			// block in Stop() until all tasks have been enqueued
			ctx, cancel := wftime.ContextWithTimeout()
			defer cancel()
			res := e.topic.Publish(ctx, &pubsub.Message{Data: jsonTask})
			if _, err := res.Get(ctx); err != nil {
				completion(fmt.Errorf("failed to publish task %+v: %w", task, err))
				return
			}
			completion(nil)
		}()
	})
}

func (e *GCPPubSubEnqueuer) Stop() {
	e.waitGroup.Wait()
}

// AWSSNSEnqueuer implements Enqueuer using AWS SNS
type AWSSNSEnqueuer struct {
	service   *sns.SNS
	topicARN  string
	waitGroup sync.WaitGroup
	dryRun    bool
}

func NewAWSSNSEnqueuer(region, identity, topicARN string, dryRun bool) (*AWSSNSEnqueuer, error) {
	session, config, err := leaws.ClientConfig(region, identity)
	if err != nil {
		return nil, err
	}

	return &AWSSNSEnqueuer{
		service:  sns.New(session, config),
		topicARN: topicARN,
		dryRun:   dryRun,
	}, nil
}

func (e *AWSSNSEnqueuer) Enqueue(task Task, completion func(error)) {
	// sns.Publish() blocks until the message has been saved by SNS, so no need
	// to asynchronously handle completion. However we still want to maintain
	// the guarantee that Stop() will block until all pending calls to Enqueue()
	// complete, so we still use a waitgroup.
	e.waitGroup.Add(1)
	defer e.waitGroup.Done()

	jsonTask, err := json.Marshal(task)
	if err != nil {
		completion(fmt.Errorf("marshaling task to JSON: %w", err))
		return
	}

	if e.dryRun {
		log.Info().Msg("dry run, not enqueuing task")
		completion(nil)
		return
	}
	// There's nothing in the PublishOutput we care about, so we discard it.
	_, err = e.service.Publish(&sns.PublishInput{
		TopicArn: aws.String(e.topicARN),
		Message:  aws.String(string(jsonTask)),
	})
	if err != nil {
		completion(fmt.Errorf("failed to publish task %+v: %w", task, err))
		return
	}

	completion(nil)
}

func (e *AWSSNSEnqueuer) Stop() {
	e.waitGroup.Wait()
}
