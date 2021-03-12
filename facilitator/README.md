# facilitator

This is ISRG's implementation of a Prio accumulation server. It ingests a share of data uploaded by a device, validates it against other facilitators, then emits the accumulated share to a Prio aggregator. It implements the subscriber portion of the `prio-server` pub/sub architecture. Each instance of `facilitator` is provided a task queue from which to dequeue tasks, which are either intake-batch or aggregate tasks.

## Getting started

[Install a Rust toolchain](https://www.rust-lang.org/tools/install), then just `cargo build|run|test`. See `cargo run --bin facilitator -- --help` for information on the various options and subcommands.

## Generating ingestion data

To generate sample ingestion data, see the `generate-ingestion-sample` command and its usage (`cargo run --bin facilitator -- generate-ingestion-sample --help`).

## Docker

To build a Docker image, run `./build.sh`. To run that image locally, `docker run letsencrypt/prio-facilitator -- --help`.

## Linting manifest files

The `facilitator lint-manifest` subcommand can validate the various manifest files used in the system. See that subcommand's help text for more information on usage.

## Working with Avro files

If you want to examine Avro-encoded messages, you can use the `avro-tools` jar from the [Apache Avro project's releases](https://downloads.apache.org/avro/avro-1.10.0/java/), and then [use it from the command line to examine individual Avro encoded objects](https://www.michael-noll.com/blog/2013/03/17/reading-and-writing-avro-files-from-the-command-line/).

## Task queues

When run with either the `intake-batch-worker` or `aggregate-worker` subcommand (or other `-worker` subcommands not yet implemented), `facilitator` runs as a persistent server whose workloop pulls tasks from a task queue. Supported task queue implementations are documented below.

### [Google PubSub](https://cloud.google.com/pubsub/docs)

Implemented in `PubSubTaskQueue` in `src/task/pubsub.rs`. `facilitator` expects that a subscription already exists and is attached to a topic to which a `workflow-manager` instance is publishing tasks. `facilitator` can share a single subscription with multiple instances of `facilitator`.

To use the PubSub task queue, pass `--task-queue-kind=gcp-pubsub` and see the program's usage for other required parameters.

Google provides a [PubSub emulator](https://cloud.google.com/pubsub/docs/emulator) useful for local testing. See the emulator documentation for information getting it set up, then simply set the `--pubsub-api-endpoint` argument to the emulator's address.

### [AWS SNS](https://docs.aws.amazon.com/sns/latest/dg/welcome.html) (**EXPERIMENTAL SUPPORT**)

Implemented in `SqsTaskQueue` in `src/task/sqs.rs`. `facilitator` expects that an SQS queue already exists and is subscribed to an SNS topic to which a `workflow-manager` instance is publishing tasks. `facilitator` can share a single SQS queue with multiple instances of `facilitator`.

AWS SNS/SQS support is experimental and has not been validated. To use it, pass `--task-queue-kind=aws-sqs` and see the program's usage for other required parameters.

### Implementing new task queues

To support new task queues, simply add an implementation of the `TaskQueue` trait, defined in `src/task.rs`. Then, add the necessary argument handling and initialization logic to `src/bin/facilitator.rs`.

## Configuration

### `aggregate-worker` thread count

The `aggregate` and `aggregate-worker` subcommands can be configured to spawn multiple worker threads that will sum over batches in an aggregation in parallel before reducing each batch's sum into a sum part that is transmitted to the portal server. By default, a single thread is used. Set either the `--thread-count` command line argument or the `THREAD_COUNT` environment variable (e.g., in a Kubernetes `ConfigMap`) to an integer value to spawn the corresponding number of threads.

To make best use of a multithreaded `aggregate-worker` in Kubernetes, make sure to configure the CPU and memory limits appropriately so that multiple cores can be allocated. For instance, a CPU request of `0.1` and a CPU limit of `3` might work well for four worker threads, keeping in mind that since Prio batch processing involves a lot of network I/O, we would not expect four worker threads on four cores to see consistently high CPU utilization.

## References

[Prio Data Share Batch IDL](https://docs.google.com/document/d/1L06dpE7OcC4CXho2UswrfHrnWKtbA9aSSmO_5o7Ku6I/edit#heading=h.3kq1yexquq2g)
[ISRG Prio server-side components design doc](https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#)
