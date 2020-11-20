variable "environment" {
  type = string
}

variable "kubernetes_namespace" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "manifest_bucket" {
  type = string
}

variable "ingestors" {
  type = list(string)
}

# We need to make a google service account for the manifest updater
resource "google_service_account" "manifest_updater" {
  provider = google-beta

  account_id   = "prio-${random_string.account_id.result}"
  display_name = "prio-${var.environment}-${var.kubernetes_namespace}-manifest-updater"
}

resource "random_string" "account_id" {
  length  = 16
  upper   = false
  number  = false
  special = false
}

# This is another kubernetes-level service account which we will associate with the operator GCP
# service account above.
resource "kubernetes_service_account" "manifest_updater" {
  metadata {
    name      = "manifest-updater"
    namespace = var.kubernetes_namespace
    annotations = {
      environment                      = var.environment
      "iam.gke.io/gcp-service-account" = google_service_account.manifest_updater.email
    }
  }
}

# This carefully constructed string lets us refer to the Kubernetes service
# account in GCP-level policies, below. See step 5 in
# https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity#authenticating_to
locals {
  manifest_updater_sa = "serviceAccount:${var.gcp_project}.svc.id.goog[${var.kubernetes_namespace}/${kubernetes_service_account.manifest_updater.metadata[0].name}]"
}

# Bind the GCP and K8s service accounts together
resource "google_service_account_iam_binding" "manifest_updater_workload" {
  provider           = google-beta
  service_account_id = google_service_account.manifest_updater.name
  role               = "roles/iam.workloadIdentityUser"
  members = [
    local.manifest_updater_sa
  ]
}

# Create a new manifest_updater role that is authorized to work with k8s secrets
resource "kubernetes_role" "manifest_updater_role" {
  metadata {
    name      = "manifest_updater"
    namespace = var.kubernetes_namespace
  }

  rule {
    api_groups = [
      ""
    ]
    resources = [
      "secrets"
    ]
    verbs = [
      "create",
      "list",
      "get",
      "delete"
    ]
  }
}

# Bind the service account we made above, with the role we made above
resource "kubernetes_role_binding" "manifest_updater_rolebinding" {
  metadata {
    name      = "${var.environment}-manifest-updater-can-update"
    namespace = var.kubernetes_namespace
  }

  role_ref {
    kind      = "Role"
    name      = kubernetes_role.manifest_updater_role.metadata[0].name
    api_group = "rbac.authorization.k8s.io"
  }

  subject {
    kind      = "ServiceAccount"
    name      = kubernetes_service_account.manifest_updater.metadata[0].name
    namespace = var.kubernetes_namespace
  }
}

# Legacy bucket writer is what we want: https://cloud.google.com/storage/docs/access-control/iam-roles
resource "google_storage_bucket_iam_member" "manifest_bucket_owner" {
  bucket = var.manifest_bucket
  role   = "roles/storage.legacyBucketWriter"
  member = "serviceAccount:${google_service_account.manifest_updater.email}"
}


locals {
  crd = yamlencode({
    "apiVersion" : "prio.isrg-prio.org/v1",
    "kind"       : "Locality",
    "metadata" : {
      "name"      : "${var.kubernetes_namespace}-locality",
      "namespace" : var.kubernetes_namespace
    },
    "spec" : {
      "environmentName"        : var.kubernetes_namespace,
      "manifestBucketLocation" : var.manifest_bucket,
      "dataShareProcessors"    : var.ingestors
      "schedule"               : "0 5 * * 0"
    }
  })
}

resource "null_resource" "crd" {
  triggers = {
    applied_crd = local.crd
    force_update = timestamp()
  }
  provisioner "local-exec" {
    command = "echo '${local.crd}\n---\n' >> crds.yml"
  }
}