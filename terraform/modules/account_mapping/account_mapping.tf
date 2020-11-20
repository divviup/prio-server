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

variable "gcp_project" {
  type = string
}

resource "random_string" "account_id" {
  length  = 16
  upper   = false
  number  = false
  special = false
}

resource "google_service_account" "account" {
  provider = google-beta

  account_id   = "prio-${random_string.account_id.result}"
  display_name = "prio-${var.google_account_name}"
}

resource "kubernetes_service_account" "account" {
  metadata {
    name      = var.kubernetes_account_name
    namespace = var.kubernetes_namespace
    annotations = {
      environment                      = var.environment
      "iam.gke.io/gcp-service-account" = google_service_account.account.email
    }
  }
}

locals {
  service_account = "serviceAccount:${var.gcp_project}.svc.id.goog[${var.kubernetes_namespace}/${kubernetes_service_account.account.metadata[0].name}]"
}

resource "google_service_account_iam_binding" "binding" {
  provider           = google-beta
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
