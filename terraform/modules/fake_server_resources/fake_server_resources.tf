variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "facilitator_manifest_base_url" {
  type = string
}

variable "ingestor_pairs" {
  type = map(object({
    locality : string
    ingestor : string
    kubernetes_namespace : string
    packet_decryption_key_kubernetes_secret : string
    ingestor_manifest_base_url : string
    peer_share_processor_manifest_base_url : string
  }))
}

variable "pushgateway" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "facilitator_image" {
  type = string
}

variable "facilitator_version" {
  type = string
}

variable "manifest_bucket" {
  type = string
}

variable "other_environment" {
  type = string
}

variable "sum_part_bucket_writer_name" {
  type = string
}

variable "sum_part_bucket_writer_email" {
  type = string
}

variable "aggregate_queues" {
  type = map(object({
    name              = string
    topic_kind        = string
    topic             = string
    subscription_kind = string
    subscription      = string
  }))
}

# TODO(brandon): this module is messy in that it conflates own/peer with facilitator/pha. (In
# practice, "own" = "facilitator" & "peer" = "pha".) This should be cleaned up to drop the own/peer
# terminology in favor of facilitator/pha.

resource "kubernetes_namespace" "tester" {
  metadata {
    name = "tester"
    annotations = {
      environment = var.environment
    }
  }
}

data "aws_caller_identity" "current" {}

resource "aws_iam_role" "tester_role" {
  name = "prio-${var.environment}-integration-tester"
  # Since azp is set in the auth token Google generates, we must check oaud in
  # the role assumption policy, and the value must match what we request when
  # requesting tokens from the GKE metadata service in
  # S3Transport::new_with_client
  # https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_iam-condition-keys.html
  assume_role_policy = <<ROLE
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "Federated": "accounts.google.com"
      },
      "Action": "sts:AssumeRoleWithWebIdentity",
      "Condition": {
        "StringEquals": {
          "accounts.google.com:sub": "${module.account_mapping.gcp_service_account_unique_id}",
          "accounts.google.com:oaud": "sts.amazonaws.com/gke-identity-federation"
        }
      }
    }
  ]
}
ROLE
}

resource "aws_iam_role_policy" "bucket_role_policy" {
  name = "prio-${var.environment}-integration-tester-policy"
  role = aws_iam_role.tester_role.id

  policy = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Action": [
        "s3:*"
      ],
      "Effect": "Allow",
      "Resource": "*"
    }
  ]
}
POLICY
}

locals {
  packet_count = 10
}

module "account_mapping" {
  source                          = "../account_mapping"
  environment                     = var.environment
  gcp_service_account_name        = "${var.environment}-fake-ingestion-identity"
  gcp_project                     = var.gcp_project
  kubernetes_service_account_name = "ingestion-identity"
  kubernetes_namespace            = kubernetes_namespace.tester.metadata[0].name
}

# Allow the integration-test identity to impersonate the sum part bucket writer
# service accounts to allow reading sum parts. (It would be more precise to
# make this read-only, but this is for testing infrastructure.)
resource "google_service_account_iam_binding" "integration_test_identity_to_my_sum_part_bucket_writer_token_creator" {
  service_account_id = var.sum_part_bucket_writer_name
  role               = "roles/iam.serviceAccountTokenCreator"
  members = [
    "serviceAccount:${module.account_mapping.gcp_service_account_email}"
  ]
}

locals {
  peer_sum_part_bucket_writer_service_account_email = "prio-${var.other_environment}-sum-writer@${var.gcp_project}.iam.gserviceaccount.com"
}

resource "google_service_account_iam_binding" "integration_test_identity_to_peer_sum_part_bucket_writer_token_creator" {
  service_account_id = "projects/${var.gcp_project}/serviceAccounts/${local.peer_sum_part_bucket_writer_service_account_email}"
  role               = "roles/iam.serviceAccountTokenCreator"
  members = [
    "serviceAccount:${module.account_mapping.gcp_service_account_email}"
  ]
}

resource "kubernetes_secret" "batch_signing_key" {
  metadata {
    generate_name = "batch-signing-key"
    namespace     = kubernetes_namespace.tester.metadata[0].name
    labels = {
      "isrg-prio.org/type" : "batch-signing-key"
    }
  }

  data = {
    # This is the base64 encoding of a PKCS#8 document containing a P-256
    # private key obtained from a development environment. Hard-coding this key
    # here absolves us of generating it elsewhere, and since it is only used in
    # test deployments, there's no hard in leaking it in git or in Terraform
    # state.
    secret_key = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgRGhfadpOZSZuMdBrjO7SqvJsJSTqTFgfDPh9bTr4MBihRANCAATq6yKefdZ2vOnp5xed74OWuHhlGW27wOvMe8cUVJ5XbS3orlg1PVmH+lHFZy7VtcGxV8WU1YAxDayZbGZ2/Te3"
  }
}

resource "google_storage_bucket_object" "global_manifest" {
  name          = "singleton-ingestor/global-manifest.json"
  bucket        = var.manifest_bucket
  content_type  = "application/json"
  cache_control = "no-cache"
  content = jsonencode({
    format = 1
    server-identity = {
      aws-iam-entity            = aws_iam_role.tester_role.arn
      gcp-service-account-id    = module.account_mapping.gcp_service_account_unique_id
      gcp-service-account-email = module.account_mapping.gcp_service_account_email
    }
    batch-signing-public-keys = {
      (kubernetes_secret.batch_signing_key.metadata[0].name) = {
        # This public key corresponds to the private key in kubernetes_secret.batch_signing_key
        public-key = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE6usinn3Wdrzp6ecXne+Dlrh4ZRlt\nu8DrzHvHFFSeV20t6K5YNT1Zh/pRxWcu1bXBsVfFlNWAMQ2smWxmdv03tw==\n-----END PUBLIC KEY-----\n"
        expiration = "2099-10-05T23:18:59Z"
      }
    }
  })
}

resource "kubernetes_deployment" "integration-test-sample-generator" {
  for_each = var.ingestor_pairs

  metadata {
    name      = "integration-test-sample-generator-${each.key}"
    namespace = kubernetes_namespace.tester.metadata[0].name
  }

  spec {
    replicas = 1
    selector {
      match_labels = {
        app      = "integration-test-sample-generator"
        locality = each.value.locality
        ingestor = each.value.ingestor
      }
    }
    template {
      metadata {
        labels = {
          app      = "integration-test-sample-generator"
          locality = each.value.locality
          ingestor = each.value.ingestor
        }
      }
      spec {
        service_account_name            = module.account_mapping.kubernetes_service_account_name
        automount_service_account_token = true
        container {
          name  = "integration-test-sample-generator"
          image = "${var.container_registry}/${var.facilitator_image}:${var.facilitator_version}"
          args = [
            "generate-ingestion-sample-worker",
            "--ingestor-name", each.value.ingestor,
            "--locality-name", each.value.locality,
            "--pha-manifest-base-url", "https://${each.value.peer_share_processor_manifest_base_url}",
            "--facilitator-manifest-base-url", "https://${var.facilitator_manifest_base_url}",
            "--batch-signing-private-key-default-identifier=${kubernetes_secret.batch_signing_key.metadata[0].name}",
            "--aggregation-id", "kittens-seen",
            "--packet-count", local.packet_count,
            "--batch-start-time", "1000000000",
            "--batch-end-time", "1000000100",
            "--dimension", "123",
            "--epsilon", "0.23",
            "--generation-interval", "60",
            "--worker-maximum-lifetime", "3600",
          ]
          env {
            name = "BATCH_SIGNING_PRIVATE_KEY"
            value_from {
              secret_key_ref {
                name     = kubernetes_secret.batch_signing_key.metadata[0].name
                key      = "secret_key"
                optional = false
              }
            }
          }
          env {
            name  = "RUST_BACKTRACE"
            value = "FULL"
          }
          env {
            name  = "RUST_LOG"
            value = "info"
          }
        }
      }
    }
  }
}

# We create a single subscription for the topic that all facilitator instances
# will dequeue tasks from. If we had multiple subscriptions, then each would see
# all the messages sent to the topic.
resource "google_pubsub_subscription" "validate" {
  for_each = var.ingestor_pairs

  name                 = "${var.environment}-${each.key}-validate"
  topic                = var.aggregate_queues[each.key].topic
  ack_deadline_seconds = 600
  # We never want the subscription to expire
  # https://registry.terraform.io/providers/hashicorp/google/latest/docs/resources/pubsub_subscription#expiration_policy
  expiration_policy {
    ttl = ""
  }
}

resource "google_pubsub_subscription_iam_binding" "validate" {
  for_each = var.ingestor_pairs

  subscription = google_pubsub_subscription.validate[each.key].name
  role         = "roles/pubsub.subscriber"
  members      = ["serviceAccount:${module.account_mapping.gcp_service_account_email}"]
}

resource "kubernetes_deployment" "integration-test-sample-validator" {
  for_each = var.ingestor_pairs

  metadata {
    name      = "integration-test-sample-validator-${each.value.locality}-${each.value.ingestor}"
    namespace = kubernetes_namespace.tester.metadata[0].name
  }

  spec {
    replicas = 1
    selector {
      match_labels = {
        app      = "integration-test-sample-validator"
        locality = each.value.locality
        ingestor = each.value.ingestor
      }
    }
    template {
      metadata {
        labels = {
          app      = "integration-test-sample-validator"
          locality = each.value.locality
          ingestor = each.value.ingestor
        }
      }
      spec {
        service_account_name            = module.account_mapping.kubernetes_service_account_name
        automount_service_account_token = true
        container {
          name  = "integration-test-sample-validator"
          image = "${var.container_registry}/${var.facilitator_image}:${var.facilitator_version}"
          args = [
            "validate-ingestion-sample-worker",
            "--instance-name=${each.key}",
            "--packet-count=${local.packet_count}",
            "--task-queue-kind=${var.aggregate_queues[each.key].subscription_kind}",
            "--task-queue-name=${google_pubsub_subscription.validate[each.key].name}",
            "--gcp-project-id=${var.gcp_project}",
            "--facilitator-identity=${var.sum_part_bucket_writer_email}",
            "--pha-identity=${local.peer_sum_part_bucket_writer_service_account_email}",
            "--pha-output=gs://prio-${var.other_environment}-sum-part-output",
            "--pha-use-default-aws-credentials-provider=true",
            "--pha-manifest-base-url=https://${each.value.peer_share_processor_manifest_base_url}",
            "--facilitator-output=gs://prio-${var.environment}-sum-part-output",
            "--facilitator-use-default-aws-credentials-provider=true",
            "--facilitator-manifest-base-url=https://${var.facilitator_manifest_base_url}",
            "--worker-maximum-lifetime=3600",
          ]
          env {
            name  = "RUST_BACKTRACE"
            value = "FULL"
          }
          env {
            name  = "RUST_LOG"
            value = "info"
          }
        }
      }
    }
  }
}

output "aws_iam_entity" {
  value = aws_iam_role.tester_role.arn
}

output "gcp_service_account_id" {
  value = module.account_mapping.gcp_service_account_unique_id
}

output "gcp_service_account_email" {
  value = module.account_mapping.gcp_service_account_email
}

output "test_kubernetes_namespace" {
  value = kubernetes_namespace.tester.metadata[0].name
}

output "batch_signing_key_name" {
  value = kubernetes_secret.batch_signing_key.metadata[0].name
}
