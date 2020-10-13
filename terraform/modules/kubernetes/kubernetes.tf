variable "peer_share_processor_name" {
  type = string
}

variable "environment" {
  type = string
}

variable "container_registry" {
  type    = string
  default = "letsencrypt"
}

variable "workflow_manager_image" {
  type = string
  # This should be "prio-data-share-processor" but we have not yet renamed the
  # container image
  default = "prio-facilitator"
}

variable "workflow_manager_version" {
  type    = string
  default = "0.1.0"
}

variable "gcp_project" {
  type = string
}

variable "ingestion_bucket" {
  type = string
}

variable "ingestion_bucket_role" {
  type = string
}

# Each data share processor is created in its own namespace
resource "kubernetes_namespace" "namespace" {
  metadata {
    name = var.peer_share_processor_name
    annotations = {
      environment = var.environment
    }
  }
}

# Workload identity[1] lets us map GCP service accounts to Kubernetes service
# accounts. We need this so that pods can use GCP API, but also AWS APIs like S3
# via Web Identity Federation. To use the credentials, the container must fetch
# the authentication token from the instance metadata service. Kubernetes has
# features for automatically providing a service account token (e.g. via a
# a mounted volume[2]), but that would be a token for the *Kubernetes* level
# service account, and not the one we can present to AWS.
# [1] https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity
# [2] https://kubernetes.io/docs/tasks/configure-pod-container/configure-service-account/#service-account-token-volume-projection

# For each data share processor, we first create a GCP service account.
resource "google_service_account" "workflow_manager" {
  provider = google-beta
  # The Account ID must be unique across the whole GCP project, and not just the
  # namespace. It must also be less than 30 characters, so we can't concatenate
  # environment and PHA name to get something unique. Instead, we generate a
  # random string.
  account_id   = "prio-${random_string.account_id.result}"
  display_name = "prio-${var.environment}-${var.peer_share_processor_name}-workflow-manager"
}

resource "random_string" "account_id" {
  length  = 16
  upper   = false
  number  = false
  special = false
}

# This is the Kubernetes-level service account which we associate with the GCP
# service account above.
resource "kubernetes_service_account" "workflow_manager" {
  metadata {
    name      = "${var.peer_share_processor_name}-workflow-manager"
    namespace = var.peer_share_processor_name
    annotations = {
      environment = var.environment
      # This annotation is necessary for the Kubernetes-GCP service account
      # mapping. See step 6 in
      # https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity#authenticating_to
      "iam.gke.io/gcp-service-account" = google_service_account.workflow_manager.email
    }
  }
}

# This carefully constructed string lets us refer to the Kubernetes service
# account in GCP-level policies, below. See step 5 in
# https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity#authenticating_to
locals {
  service_account = "serviceAccount:${var.gcp_project}.svc.id.goog[${kubernetes_namespace.namespace.metadata[0].name}/${kubernetes_service_account.workflow_manager.metadata[0].name}]"
}

# Allows the Kubernetes service account to impersonate the GCP service account.
resource "google_service_account_iam_binding" "workflow_manager_workload" {
  provider           = google-beta
  service_account_id = google_service_account.workflow_manager.name
  role               = "roles/iam.workloadIdentityUser"
  members = [
    local.service_account
  ]
}

# Allows the Kubernetes service account to request auth tokens for the GCP
# service account.
resource "google_service_account_iam_binding" "workflow_manager_token" {
  provider           = google-beta
  service_account_id = google_service_account.workflow_manager.name
  role               = "roles/iam.serviceAccountTokenCreator"
  members = [
    local.service_account
  ]
}

resource "kubernetes_secret" "batch_signing_key" {
  metadata {
    name      = "${var.environment}-${var.peer_share_processor_name}-batch-signing-key"
    namespace = var.peer_share_processor_name
  }

  data = {
    # We want this to be a Terraform resource that can be managed and destroyed
    # by this module, but we do not want the cleartext private key to appear in
    # the TF statefile. So we set a dummy value here, and will update the value
    # later using kubectl. We use lifecycle.ignore_changes so that Terraform
    # won't blow away the replaced value on subsequent applies.
    signing_key = "not-a-real-key"
  }

  lifecycle {
    ignore_changes = [
      data["signing_key"]
    ]
  }
}

resource "kubernetes_secret" "ingestion_packet_decryption_key" {
  metadata {
    name      = "${var.environment}-${var.peer_share_processor_name}-ingestion-packet-decryption-key"
    namespace = var.peer_share_processor_name
  }

  data = {
    # See comment on batch_signing_key, above, about the initial value here.
    decryption_key = "not-a-real-key"
  }

  lifecycle {
    ignore_changes = [
      data["decryption_key"]
    ]
  }
}

resource "kubernetes_cron_job" "workflow_manager" {
  metadata {
    name      = "${var.environment}-workflow-manager"
    namespace = var.peer_share_processor_name

    annotations = {
      environment = var.environment
    }
  }
  spec {
    schedule                  = "*/10 * * * *"
    failed_jobs_history_limit = 3
    job_template {
      metadata {}
      spec {
        template {
          metadata {}
          spec {
            container {
              name  = "facilitator"
              image = "${var.container_registry}/${var.workflow_manager_image}:${var.workflow_manager_version}"
              # Write sample data to exercise writing into S3.
              args = [
                "generate-ingestion-sample",
                "--aggregation-id", "fake-1",
                "--batch-id", "eb03ef04-5f05-4a64-95b2-ca1b841b6885",
                "--date", "2020/09/11/21/11",
                "--facilitator-output", "s3://${var.ingestion_bucket}",
                "--pha-output", "/tmp/sample-pha"
              ]
              env {
                name  = "AWS_ROLE_ARN"
                value = var.ingestion_bucket_role
              }
              env {
                name = "BATCH_SIGNING_KEY"
                value_from {
                  secret_key_ref {
                    name = kubernetes_secret.batch_signing_key.metadata[0].name
                    key  = "signing_key"
                  }
                }
              }
              env {
                name = "INGESTION_PACKET_DECRYPTION_KEY"
                value_from {
                  secret_key_ref {
                    name = kubernetes_secret.ingestion_packet_decryption_key.metadata[0].name
                    key  = "decryption_key"
                  }
                }
              }
            }
            # If we use any other restart policy, then when the job is finally
            # deemed to be a failure, Kubernetes will destroy the job, pod and
            # container(s) virtually immediately. This can cause us to lose logs
            # if the container is reaped before the GKE logging agent can upload
            # logs. Since this is a cronjob and we will retry anyway, we use
            # "Never".
            # https://kubernetes.io/docs/concepts/workloads/controllers/job/#handling-pod-and-container-failures
            # https://github.com/kubernetes/kubernetes/issues/74848
            restart_policy       = "Never"
            service_account_name = kubernetes_service_account.workflow_manager.metadata[0].name
          }
        }
      }
    }
  }
}

output "service_account_unique_id" {
  value = google_service_account.workflow_manager.unique_id
}

output "batch_signing_key" {
  value = kubernetes_secret.batch_signing_key.metadata[0].name
}

output "ingestion_packet_decryption_key" {
  value = kubernetes_secret.ingestion_packet_decryption_key.metadata[0].name
}
