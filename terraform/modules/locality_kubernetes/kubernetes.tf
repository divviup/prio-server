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

module "account_mapping" {
  source = "../account_mapping"
  google_account_name = "${var.environment}-${var.kubernetes_namespace}-manifest-updater"
  kubernetes_account_name = "manifest-updater"
  kubernetes_namespace = var.kubernetes_namespace
  environment = var.environment
  gcp_project = var.gcp_project
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
    name      = module.account_mapping.kubernetes_account_name
    namespace = var.kubernetes_namespace
  }
}

# Legacy bucket writer is what we want: https://cloud.google.com/storage/docs/access-control/iam-roles
resource "google_storage_bucket_iam_member" "manifest_bucket_owner" {
  bucket = var.manifest_bucket
  role   = "roles/storage.legacyBucketWriter"
  member = "serviceAccount:${module.account_mapping.google_service_account_email}"
}


locals {
  crd = yamlencode({
    "apiVersion" : "prio.isrg-prio.org/v1",
    "kind" : "Locality",
    "metadata" : {
      "name" : "${var.kubernetes_namespace}-locality",
      "namespace" : var.kubernetes_namespace
    },
    "spec" : {
      "environmentName" : var.kubernetes_namespace,
      "manifestBucketLocation" : var.manifest_bucket,
      "ingestors" : var.ingestors
      "schedule" : "0 5 * * 0"
    }
  })
}

resource "null_resource" "crd" {
  triggers = {
    applied_crd = local.crd
  }
  provisioner "local-exec" {
    command = "echo '${local.crd}\n---\n' >> crds.yml"
  }
}
