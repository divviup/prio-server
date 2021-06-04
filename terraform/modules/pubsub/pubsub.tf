variable "environment" {
  type = string
}

variable "data_share_processor_name" {
  type = string
}

variable "publisher_service_account" {
  type = string
}

variable "subscriber_service_account" {
  type = string
}

variable "task" {
  type = string
}

data "google_project" "project" {}

locals {
  # GCP PubSub creates a service account for each project used to move
  # undeliverable messages from subscriptons to a dead letter topic
  # https://cloud.google.com/pubsub/docs/dead-letter-topics#granting_forwarding_permissions
  pubsub_service_account = "serviceAccount:service-${data.google_project.project.number}@gcp-sa-pubsub.iam.gserviceaccount.com"
}

resource "google_pubsub_topic" "task" {
  name = "${var.environment}-${var.data_share_processor_name}-${var.task}"
}

resource "google_pubsub_topic_iam_binding" "task" {
  topic = google_pubsub_topic.task.name
  role  = "roles/pubsub.publisher"
  members = [
    "serviceAccount:${var.publisher_service_account}"
  ]
}

# We create a single subscription for the topic that all facilitator instances
# will dequeue tasks from. If we had multiple subscriptions, then each would see
# all the messages sent to the topic.
resource "google_pubsub_subscription" "task" {
  name                 = "${var.environment}-${var.data_share_processor_name}-${var.task}"
  topic                = google_pubsub_topic.task.name
  ack_deadline_seconds = 600
  # We never want the subscription to expire
  # https://registry.terraform.io/providers/hashicorp/google/latest/docs/resources/pubsub_subscription#expiration_policy
  expiration_policy {
    ttl = ""
  }
  dead_letter_policy {
    dead_letter_topic     = google_pubsub_topic.dead_letter.id
    max_delivery_attempts = 5
  }
}

resource "google_pubsub_subscription_iam_binding" "task" {
  subscription = google_pubsub_subscription.task.name
  role         = "roles/pubsub.subscriber"
  members = [
    "serviceAccount:${var.subscriber_service_account}",
    local.pubsub_service_account
  ]
}

# Dead letter topic to which undeliverable intake batch messages are sent
resource "google_pubsub_topic" "dead_letter" {
  name = "${google_pubsub_topic.task.name}-dead-letter"
}

resource "google_pubsub_topic_iam_binding" "dead_letter" {
  topic = google_pubsub_topic.dead_letter.name
  role  = "roles/pubsub.publisher"
  members = [
    local.pubsub_service_account
  ]
}

# Subscription on the dead letter topic enabling us to alert on dead letter
# delivery and examine undelivered messages
resource "google_pubsub_subscription" "dead_letter" {
  name                 = google_pubsub_topic.dead_letter.name
  topic                = google_pubsub_topic.dead_letter.name
  ack_deadline_seconds = 600
  # Subscription should never expire
  expiration_policy {
    ttl = ""
  }
}

output "topic" {
  value = google_pubsub_topic.task.name
}

output "subscription" {
  value = google_pubsub_subscription.task.name
}
