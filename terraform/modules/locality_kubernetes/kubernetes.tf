variable "environment" {
  type = string
}

variable "kubernetes_namespace" {
  type = string
}

variable "manifest_bucket" {
  type = string
}

variable "ingestors" {
  type = list(string)
}

variable "batch_signing_key_expiration" {
  type = number
}

variable "batch_signing_key_rotation" {
  type = number
}

variable "packet_encryption_key_expiration" {
  type = number
}

variable "packet_encryption_rotation" {
  type = number
}

variable "use_aws" {
  type = bool
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

module "account_mapping" {
  source                          = "../account_mapping"
  gcp_service_account_name        = var.use_aws ? "" : "${var.environment}-${var.kubernetes_namespace}-manifest-updater"
  aws_iam_role_name               = var.use_aws ? "${var.environment}-${var.kubernetes_namespace}-manifest-updater" : ""
  oidc_provider                   = var.oidc_provider
  kubernetes_service_account_name = "manifest-updater"
  kubernetes_namespace            = var.kubernetes_namespace
  environment                     = var.environment
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
    name      = module.account_mapping.kubernetes_service_account_name
    namespace = var.kubernetes_namespace
  }
}

resource "google_storage_bucket_iam_member" "manifest_bucket_writer" {
  count = var.use_aws ? 0 : 1

  bucket = var.manifest_bucket
  role   = "roles/storage.legacyBucketWriter"
  member = "serviceAccount:${module.account_mapping.gcp_service_account_email}"
}

data "aws_s3_bucket" "manifest_bucket" {
  count  = var.use_aws ? 1 : 0
  bucket = var.manifest_bucket
}

# Allow the IAM role to write and replace objects in the manifest bucket
resource "aws_iam_role_policy" "manifest_bucket_writer" {
  count = var.use_aws ? 1 : 0

  name = "prio-${var.environment}-${var.kubernetes_namespace}-manifest-updater"
  role = module.account_mapping.aws_iam_role_name
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "PutObject"
        Effect = "Allow"
        Action = [
          "s3:PutObject",
          "s3:PutObjectAcl",
          "s3:GetObject",
          "s3:GetObjectAcl",
          "s3:DeleteObjectAcl",
        ]
        Resource = data.aws_s3_bucket.manifest_bucket[0].arn
      }
    ]
  })
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
      "ingestors" : var.ingestors,
      "schedule" : "0 5 * * 0",
      "batchSigningKeySpec" : {
        "keyValidity" : var.batch_signing_key_expiration
        "keyRotationInterval" : var.batch_signing_key_rotation
      },
      "packetEncryptionKeySpec" : {
        "keyValidity" : var.packet_encryption_key_expiration
        "keyRotationInterval" : var.packet_encryption_rotation
      }
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
