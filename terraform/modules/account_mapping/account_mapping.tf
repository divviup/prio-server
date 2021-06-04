variable "gcp_service_account_name" {
  type    = string
  default = ""
}

variable "aws_iam_role_name" {
  type    = string
  default = ""
}

variable "oidc_provider" {
  type = object({
    arn = string
    url = string
  })
  default = {
    arn = ""
    url = ""
  }
}

variable "kubernetes_service_account_name" {
  type = string
}

variable "kubernetes_namespace" {
  type = string
}

variable "environment" {
  type = string
}

variable "allow_gcp_sa_token_creation" {
  type    = bool
  default = false
}

data "google_project" "current" {}

resource "random_string" "account_id" {
  length  = 16
  upper   = false
  number  = false
  special = false
}

# GCP GKE and AWS EKS both allow mapping Kubernetes service accounts to an
# entity in the underlying cloud platform's IAM system, which we need so that
# Kubernetes pods can use GCP or AWS APIs like S3, PubSub, etc. Kubernetes has
# features for automatically providing a service account token (e.g. via a
# a mounted volume[1]), but that would be a token for the *Kubernetes* level
# service account, and not the one we can present to AWS or GCP.
# On GCP, we use Workload Identity[1] to map a Kubernetes SA to a GCP SA. On
# AWS,  we use IAM roles for service accounts[3] to map a Kubernetes SA to an
# AWS IAM role.
#
# This module creates the Kubernetes SA and the cloud platform IAM entity and
# maps one to the other. It is the caller's responsibility to then associate
# whatever other policies, roles, etc. are necessary to either resource.
#
# [1] https://kubernetes.io/docs/tasks/configure-pod-container/configure-service-account/#service-account-token-volume-projection
# [2] https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity
# [3] https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html

# We first create a GCP service account, if provided an account name
resource "google_service_account" "account" {
  count = var.gcp_service_account_name != "" ? 1 : 0

  # The Account ID must be unique across the whole GCP project, and not just the
  # namespace. It must also be fewer than 30 characters, so we can't concatenate
  # environment and PHA name to get something unique. Instead, we generate a
  # random string.
  account_id   = "prio-${random_string.account_id.result}"
  display_name = "prio-${var.gcp_service_account_name}"
}

# We first create a AWS IAM role, if a role name was provided
resource "aws_iam_role" "iam_role" {
  count = var.aws_iam_role_name != "" ? 1 : 0

  name = var.aws_iam_role_name

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = "sts:AssumeRoleWithWebIdentity"
        Principal = {
          Federated = var.oidc_provider.arn
        }
        Condition = {
          StringEquals = {
            "${var.oidc_provider.url}:sub" = "system:serviceaccount:${var.kubernetes_namespace}:${var.kubernetes_service_account_name}"
          }
        }
      }
    ]
  })
}

# This is the Kubernetes-level service account which we associate with the GCP
# service account or AWS IAM role created above.
resource "kubernetes_service_account" "account" {
  metadata {
    name      = var.kubernetes_service_account_name
    namespace = var.kubernetes_namespace
    annotations = var.gcp_service_account_name != "" ? {
      environment                      = var.environment
      "iam.gke.io/gcp-service-account" = google_service_account.account[0].email
      } : {
      environment                  = var.environment
      "eks.amazonaws.com/role-arn" = aws_iam_role.iam_role[0].arn
    }
  }
}

# This carefully constructed string lets us refer to the Kubernetes service
# account in GCP-level policies, below. See step 5 in
# https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity#authenticating_to
locals {
  service_account_gcp_role_member = "serviceAccount:${data.google_project.current.project_id}.svc.id.goog[${var.kubernetes_namespace}/${kubernetes_service_account.account.metadata[0].name}]"
}

# Allows the Kubernetes service account to impersonate the GCP service account.
resource "google_service_account_iam_binding" "binding" {
  count = var.gcp_service_account_name != "" ? 1 : 0

  service_account_id = google_service_account.account[0].name
  role               = "roles/iam.workloadIdentityUser"
  members = [
    local.service_account_gcp_role_member
  ]
}

# Allows the Kubernetes service account to request auth tokens for the GCP
# service account.
resource "google_service_account_iam_binding" "workflow_manager_token" {
  count = (var.allow_gcp_sa_token_creation && var.gcp_service_account_name != "") ? 1 : 0

  service_account_id = google_service_account.account[0].name
  role               = "roles/iam.serviceAccountTokenCreator"
  members = [
    local.service_account_gcp_role_member
  ]
}

output "gcp_service_account_name" {
  value = var.gcp_service_account_name != "" ? google_service_account.account[0].name : ""
}

output "gcp_service_account_email" {
  value = var.gcp_service_account_name != "" ? google_service_account.account[0].email : ""
}

output "gcp_service_account_unique_id" {
  value = var.gcp_service_account_name != "" ? google_service_account.account[0].unique_id : ""
}

output "kubernetes_service_account_name" {
  value = kubernetes_service_account.account.metadata[0].name
}

output "aws_iam_role_name" {
  value = var.aws_iam_role_name != "" ? aws_iam_role.iam_role[0].name : ""
}

output "aws_iam_role_arn" {
  value = var.aws_iam_role_name != "" ? aws_iam_role.iam_role[0].arn : ""
}
