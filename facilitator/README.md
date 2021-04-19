# facilitator

This is ISRG's implementation of a Prio accumulation server. It ingests a share of data uploaded by a device, validates it against other facilitators, then emits the accumulated share to a Prio aggregator. It implements the subscriber portion of the `prio-server` pub/sub architecture. Each instance of `facilitator` is provided a task queue from which to dequeue tasks, which are either intake-batch or aggregate tasks.

## Getting started

[Install a Rust toolchain](https://www.rust-lang.org/tools/install), then just `cargo build|run|test`. See `cargo run --bin facilitator -- --help` for information on the various options and subcommands.

## Simulating a protocol run with sample data

`facilitator` is capable of running offline, reading and writing to local paths, and it can generate random data. To simulate a run of the protocol locally, try:

    cargo run -- generate-ingestion-sample \
        --aggregation-id test-aggregation \
        --dimension 10 \
        --epsilon 0.11 \
        --batch-end-time 100 \
        --batch-start-time 100 \
        --packet-count 10 \
        --batch-signing-private-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5bWIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxouP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs" \
        --batch-signing-private-key-identifier="signing-key-1" \
        --facilitator-output=/tmp/facil-out \
        --facilitator-ecies-public-key="BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g/WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w==" \
        --peer-output /tmp/pha-out \
        --pha-ecies-public-key="BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw=="

This will generate a sample batch containing 10 packets, using the current time as a timestamp. The `batch-signing-private-key` argument is the base64 encoding of the DER encoding of an ECDSA P256 private key. The `facilitator-ecies-public-key` and `pha-ecies-public-key` arguments are the base64 encoding of the uncompressed X9.62 representation of either the private or public key.

To simulate intake on the facilitator server:

    cargo run -- intake-batch \
        --aggregation-id test-aggregation \
        --batch-id ba097344-2b4e-45db-a002-c83f4a9adc63 \
        --date 2021/04/13/19/17 \
        --instance-name="local-test" \
        --is-first=false \
        --batch-signing-private-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeSa+S+tmLupnAEyFKdVuKB99y09YEqW41+8pwP4cTkahRANCAASy7FHcLGnRudVHWga/j2k9nQ3lMvuGE01Q7DEyjyCuuw9YmB3dHvYcRUnxVRI/nF5LvneGim0dC7F1fuRAPeXI" \
        --batch-signing-private-key-identifier=facil-signing-key \
        --ingestor-input /tmp/facil-out/ \
        --ingestor-public-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5bWIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxouP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs" \
        --ingestor-public-key-identifier="signing-key-1" \
        --own-output=/tmp/facil-own-validation \
        --peer-output=/tmp/facil-peer-validation \
        --packet-decryption-keys="BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g/WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w=="

Note that `batch-id` and `date` must correspond to the batch that was emitted by `generate-ingestion-sample`, `ingestor-public-key` and `ingestor-public-key-identifier` must match the values used for `batch-signing-private-key` and `batch-signing-private-key-identifier` and that `packet-decryption-keys` must match `facilitator-ecies-public-key`.

To simulate intake on the PHA server:

    cargo run -- intake-batch \
        --aggregation-id test-aggregation \
        --batch-id ba097344-2b4e-45db-a002-c83f4a9adc63 \
        --date 2021/04/13/19/17 \
        --instance-name="local-test" \
        --is-first=true \
        --batch-signing-private-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg1BQjH71U37XLfWqe+/xP8iUrMiHpmUtbj3UfDkhFIrShRANCAAQgqHcxxwTVx1IXimcRv5TQyYZh+ShDM6XZqJonoP1m52oN0aLID1hJSrfKJrnqdgmHmaT4eXNNf4C5+g1HZt+u" \
        --batch-signing-private-key-identifier=pha-signing-key \
        --ingestor-input /tmp/pha-out/ \
        --ingestor-public-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5bWIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxouP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs" \
        --ingestor-public-key-identifier="signing-key-1" \
        --own-output=/tmp/pha-own-validation \
        --peer-output=/tmp/pha-peer-validation \
        --packet-decryption-keys="BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw=="

Note again the correspondence with the arguments passed to `generate-ingestion-sample`. Note also that this time we set `is-first=true`.

To simulate aggregation on the facilitator server:

    cargo run -- aggregate \
        --aggregation-id test-aggregation \
        --aggregation-start 2021/04/13/19/17 \
        --aggregation-end 2021/04/13/19/17 \
        --batch-signing-private-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeSa+S+tmLupnAEyFKdVuKB99y09YEqW41+8pwP4cTkahRANCAASy7FHcLGnRudVHWga/j2k9nQ3lMvuGE01Q7DEyjyCuuw9YmB3dHvYcRUnxVRI/nF5LvneGim0dC7F1fuRAPeXI" \
        --batch-signing-private-key-identifier=facil-signing-key \
        --ingestor-input /tmp/facil-out/ \
        --ingestor-public-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5bWIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxouP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs" \
        --ingestor-public-key-identifier="signing-key-1" \
        --packet-decryption-keys="BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g/WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w==" \
        --instance-name="local-test" \
        --is-first=false \
        --own-input /tmp/facil-own-validation \
        --peer-input /tmp/pha-peer-validation \
        --peer-public-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg1BQjH71U37XLfWqe+/xP8iUrMiHpmUtbj3UfDkhFIrShRANCAAQgqHcxxwTVx1IXimcRv5TQyYZh+ShDM6XZqJonoP1m52oN0aLID1hJSrfKJrnqdgmHmaT4eXNNf4C5+g1HZt+u" \
        --peer-public-key-identifier=pha-signing-key \
        --portal-output /tmp/facil-sum-parts \
        --batch-time 2021/04/13/19/17 \
        --batch-id ba097344-2b4e-45db-a002-c83f4a9adc63

Had you generated and intaken multiple batches with `generate-ingestion-sample` and `intake-batch`, you could pass `batch-time` and `batch-id` multiple times. To simulate aggregation on the PHA server:

    cargo run -- aggregate \
        --aggregation-id test-aggregation \
        --aggregation-start 2021/04/13/19/17 \
        --aggregation-end 2021/04/13/19/17 \
        --batch-signing-private-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg1BQjH71U37XLfWqe+/xP8iUrMiHpmUtbj3UfDkhFIrShRANCAAQgqHcxxwTVx1IXimcRv5TQyYZh+ShDM6XZqJonoP1m52oN0aLID1hJSrfKJrnqdgmHmaT4eXNNf4C5+g1HZt+u" \
        --batch-signing-private-key-identifier=pha-signing-key \
        --ingestor-input /tmp/pha-out/ \
        --ingestor-public-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5bWIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxouP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs" \
        --ingestor-public-key-identifier="signing-key-1" \
        --packet-decryption-keys="BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==" \
        --instance-name="local-test" \
        --is-first=true \
        --own-input /tmp/pha-own-validation \
        --peer-input /tmp/facil-peer-validation \
        --peer-public-key="MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeSa+S+tmLupnAEyFKdVuKB99y09YEqW41+8pwP4cTkahRANCAASy7FHcLGnRudVHWga/j2k9nQ3lMvuGE01Q7DEyjyCuuw9YmB3dHvYcRUnxVRI/nF5LvneGim0dC7F1fuRAPeXI" \
        --peer-public-key-identifier=facil-signing-key \
        --portal-output /tmp/pha-sum-parts \
        --batch-time 2021/04/13/19/17 \
        --batch-id ba097344-2b4e-45db-a002-c83f4a9adc63

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

### [AWS SNS](https://docs.aws.amazon.com/sns/latest/dg/welcome.html)

Implemented in `SqsTaskQueue` in `src/task/sqs.rs`. `facilitator` expects that an SQS queue already exists and is subscribed to an SNS topic to which a `workflow-manager` instance is publishing tasks. `facilitator` can share a single SQS queue with multiple instances of `facilitator`. `facilitator` does not expect messages wrapped in metadata, and so [raw message delivery](https://docs.aws.amazon.com/sns/latest/dg/sns-large-payload-raw-message-delivery.html) should be enabled when configuring SQS queues.

To use it, pass `--task-queue-kind=aws-sqs` and see the program's usage for other required parameters.

### Implementing new task queues

To support new task queues, simply add an implementation of the `TaskQueue` trait, defined in `src/task.rs`. Then, add the necessary argument handling and initialization logic to `src/bin/facilitator.rs`.

## References

[Prio Data Share Batch IDL](https://docs.google.com/document/d/1L06dpE7OcC4CXho2UswrfHrnWKtbA9aSSmO_5o7Ku6I/edit#heading=h.3kq1yexquq2g)
[ISRG Prio server-side components design doc](https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#)
