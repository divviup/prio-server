# task-replayer

A simple utility for replaying tasks into the system from a file containing the JSON encoding of a
task. To build or run it, install a Go toolchain and then, from the `task-replayer/` directory:

    go build


Tasks can be obtained from a GCP PubSub dead letter queue using `gcloud`:

    gcloud --project=prio-prod-us pubsub subscriptions pull projects/YOUR_GCP_PROJECT_NAME/subscriptions/DEAD_LETTER_SUBSCRIPTION_NAME --format=json

You may have to try multiple times before anything is pulled from the subscription. The response will be a list of objects, which contain a `.message.data` key. That is the base64 encoding of the JSON encoding of the task. So, as a one-liner:

    gcloud --project=prio-prod-us pubsub subscriptions pull projects/YOUR_GCP_PROJECT_NAME/subscriptions/DEAD_LETTER_SUBSCRIPTION_NAME --format=json | jq -r '.[0].message.data' | base64 -d > /tmp/task.json

If a task was successfully pulled from the DLQ, then:

    /path/to/compiled/task-replayer --gcp-project-id YOUR_GCP_PROJECT_NAME --task-queue-kind gcp-pubsub --topic TASK_TOPIC_NAME --aggregate-task /tmp/task.json

Note that `--topic` should be the name of the topic within the project, and not the `project/<name>/topics/<name>` format `gcloud` might expect.
