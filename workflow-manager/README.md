# workflow-manager

`workflow-manager` is the publisher portion of the Prio pub/sub architecture. It is responsible for:

- scanning the contents of ingestion, own validation and peer validation buckets and translating the objects it finds into intake and aggregation tasks
- writing markers to avoid scheduling duplicate tasks
- reaping Kubernetes jobs left behind by older versions of itself

## Task queues

`workflow-manager` schedules work by sending messages into a queue, which are later consumed by `facilitator` worker instances. We currently support the following message queues:

### [Google PubSub](https://cloud.google.com/pubsub/docs)

Implemented in `GCPPubSubEnqueuer` in `task/task.go`. The model is that each `workflow-manager` instance uses distinct topics for intake and aggregation tasks, so at a higher level, there are distinct PubSub topics for each (locality, ingestor, task) tuple. There is one subscription for each topic, shared among pools of `intake-batch-worker` and `aggregate-worker` instances of `facilitator`. `workflow-manager` assumes that PubSub topics with appropriate names, permissions and subscriptions already exist.

Google provides a [PubSub emulator](https://cloud.google.com/pubsub/docs/emulator) useful for local testing. See the emulator documentation for information getting it set up, then simply set the `PUBSUB_EMULATOR_HOST` environment variable to the emulator's address when running `workflow-manager`.

`workflow-manager` expects the topics to which it writes messages to already have been created in Terraform, and `gcloud` cannot be used to interact with the emulator, so `workflow-manager` takes the `--create-pubsub-topics` flag. When set, `workflow-manager` will create topics with the names provided to the `--intake-tasks-topic` and `--aggregate-tasks-topic` parameters before doing any work.

### [AWS SNS](https://docs.aws.amazon.com/sns/latest/dg/welcome.html)

Implemented in `AWSSNSEnqueuer` in `task/task.go`. The model here is that each `workflow-manager` instance uses distinct SNS topics for intake and aggregation tasks, so at a higher level, there are distinct SNS topics for each (locality, ingestor, task) tuple. It is assumed that there is one SQS queue for each topic, shared among pools of `intake-batch-worker` and `aggregate-worker` instances of `facilitator`. `workflow-manager` assumes that SNS topics and SQS queues with appropriate names, permissions and configurations already exist. `facilitator` does not expect messages wrapped in SQS metadata, and so [raw message delivery](https://docs.aws.amazon.com/sns/latest/dg/sns-large-payload-raw-message-delivery.html) should be enabled on SQS queues.

To use it, invoke `workflow-manager` with `--task-queue-kind=aws-sns`, and provide other `--aws-sns-` parameters as appropriate for your deployment.

### Implementing new task queues

To support new task queues, simply add an implementation of the `task.Enqueuer` interface in `task/task.go`. Then, add the necessary argument handling and initialization logic to `main.go` as directed by the comments there.

## Developing and debugging

`workflow-manager` is intended to run as a Kubernetes cronjob, but it can also be run locally from the command line. It uses cloud platform APIs to access storage buckets and Kubernetes APIs to list and manipulate jobs. If no special arguments are provided, it will use ambient cloud platform credits (i.e., whatever is in `~/.aws` or `~/.config/gcloud`) for the former. For Kubernetes API access, it defaults to using the in cluster client configuration, expecting a Kubernetes service account with appropriate RBAC permissions to be mounted. However you can have it use the credentials configured for `kubectl` by passing `--kube-config-path /path/to/your/.kube/config`. See `kubectl` documentation for more information on `kubeconfig`.

### Dry run mode

If you want to see what `workflow-manager` would do and avoid any side-effects, pass `--dry-run`. No tasks will be scheduled, no objects will be written to cloud storage and no Kubernetes jobs will be deleted. Instead, the operations that would have been performed will be logged.

Note that dry run mode does not guarantee that the logged operations would have succeeded.
