variable "google_account_name" {
  type = string
}

variable "kubernetes_account_name" {
  type = string
}

variable "kubernetes_namespace" {
  type = string
}

variable "environment" {
  type = string
}

data "google_project" "current" {}

resource "random_string" "account_id" {
  length  = 16
  upper   = false
  number  = false
  special = false
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

# We first create a GCP service account.
resource "google_service_account" "account" {
  # The Account ID must be unique across the whole GCP project, and not just the
  # namespace. It must also be fewer than 30 characters, so we can't concatenate
  # environment and PHA name to get something unique. Instead, we generate a
  # random string.
  account_id   = "prio-${random_string.account_id.result}"
  display_name = "prio-${var.google_account_name}"
}


# This is the Kubernetes-level service account which we associate with the GCP
# service account above.
resource "kubernetes_service_account" "account" {
  automount_service_account_token = false
  metadata {
    name      = var.kubernetes_account_name
    namespace = var.kubernetes_namespace
    annotations = {
      environment                      = var.environment
      "iam.gke.io/gcp-service-account" = google_service_account.account.email
    }
  }
}

# This carefully constructed string lets us refer to the Kubernetes service
# account in GCP-level policies, below. See step 5 in
# https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity#authenticating_to
locals {
  service_account = "serviceAccount:${data.google_project.current.project_id}.svc.id.goog[${var.kubernetes_namespace}/${kubernetes_service_account.account.metadata[0].name}]"
}

# Allows the Kubernetes service account to impersonate the GCP service account.
resource "google_service_account_iam_binding" "binding" {
  service_account_id = google_service_account.account.name
  role               = "roles/iam.workloadIdentityUser"
  members = [
    local.service_account
  ]
}

output "google_service_account_name" {
  value = google_service_account.account.name
}

output "google_service_account_email" {
  value = google_service_account.account.email
}

output "google_service_account_unique_id" {
  value = google_service_account.account.unique_id
}

output "kubernetes_account_name" {
  value = kubernetes_service_account.account.metadata[0].name
}

output "service_account" {
  value = local.service_account
}
