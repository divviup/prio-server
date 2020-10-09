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

variable "execution_manager_image" {
  type    = string
  default = "prio-facilitator"
}

variable "execution_manager_version" {
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

# Each facilitator is created in its own namespace
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

# For each facilitator, we first create a GCP service account.
resource "google_service_account" "execution_manager" {
  provider = google-beta
  # The Account ID must be unique across the whole GCP project, and not just the
  # namespace. It must also be less than 30 characters, so we can't concatenate
  # environment and PHA name to get something unique. Instead, we generate a
  # random string.
  account_id   = "prio-${random_string.account_id.result}"
  display_name = "prio-${var.environment}-${var.peer_share_processor_name}-execution-manager"
}

resource "random_string" "account_id" {
  length  = 16
  upper   = false
  number  = false
  special = false
}

# This is the Kubernetes-level service account which we associate with the GCP
# service account above.
resource "kubernetes_service_account" "execution_manager" {
  metadata {
    name      = "${var.peer_share_processor_name}-execution-manager"
    namespace = var.peer_share_processor_name
    annotations = {
      environment = var.environment
      # This annotation is necessary for the Kubernetes-GCP service account
      # mapping. See step 6 in
      # https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity#authenticating_to
      "iam.gke.io/gcp-service-account" = google_service_account.execution_manager.email
    }
  }
}

# This carefully constructed string lets us refer to the Kubernetes service
# account in GCP-level policies, below. See step 5 in
# https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity#authenticating_to
locals {
  service_account = "serviceAccount:${var.gcp_project}.svc.id.goog[${kubernetes_namespace.namespace.metadata[0].name}/${kubernetes_service_account.execution_manager.metadata[0].name}]"
}

# Allows the Kubernetes service account to impersonate the GCP service account.
resource "google_service_account_iam_binding" "execution-manager-workload" {
  provider           = google-beta
  service_account_id = google_service_account.execution_manager.name
  role               = "roles/iam.workloadIdentityUser"
  members = [
    local.service_account
  ]
}

# Allows the Kubernetes service account to request auth tokens for the GCP
# service account.
resource "google_service_account_iam_binding" "execution-manager-token" {
  provider           = google-beta
  service_account_id = google_service_account.execution_manager.name
  role               = "roles/iam.serviceAccountTokenCreator"
  members = [
    local.service_account
  ]
}

resource "kubernetes_cron_job" "execution_manager" {
  metadata {
    name      = "${var.environment}-execution-manager"
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
              image = "${var.container_registry}/${var.execution_manager_image}:${var.execution_manager_version}"
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
            service_account_name = kubernetes_service_account.execution_manager.metadata[0].name
          }
        }
      }
    }
  }
}

output "service_account_unique_id" {
  value = google_service_account.execution_manager.unique_id
}
