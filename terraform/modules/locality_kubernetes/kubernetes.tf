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

variable "manifest_bucket_base_url" {
  type = string
}

variable "certificate_domain" {
  type = string
}

variable "ingestors" {
  type        = map(string)
  description = "Map of ingestor names to the URL where their global manifest may be found."
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

module "account_mapping" {
  source                  = "../account_mapping"
  google_account_name     = "${var.environment}-${var.kubernetes_namespace}-manifest-updater"
  kubernetes_account_name = "manifest-updater"
  kubernetes_namespace    = var.kubernetes_namespace
  environment             = var.environment
  gcp_project             = var.gcp_project
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
      "ingestors" : keys(var.ingestors),
      "schedule" : "0 5 * * 0",
      "fqdn": "${var.kubernetes_namespace}.${var.certificate_domain}",
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


variable "peer_share_processor_manifest_base_url" {
  type = string
}

variable "use_aws" {
  type = bool
}

variable "aws_region" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "google_service_account_sum_part_bucket_writer_email" {
  type = string
}

variable "google_service_account_sum_part_bucket_writer_name" {
  type = string
}

variable "portal_server_manifest_base_url" {
  type = string
}

variable "test_peer_environment" {
  type        = map(string)
  default     = {}
  description = "See main.tf for discussion."
}

variable "is_first" {
  type = bool
}

variable "aggregation_period" {
  type = string
}

variable "aggregation_grace_period" {
  type = string
}

variable "kms_keyring" {
  type = string
}

# For each peer data share processor, we will receive ingestion batches from two
# ingestion servers. We create a distinct data share processor instance for each
# (peer, ingestor) pair.
# First, we fetch the ingestor global manifests, which yields a map of ingestor
# name => HTTP content.
data "http" "ingestor_global_manifests" {
  for_each = var.ingestors
  url      = "https://${each.value}/global-manifest.json"
}

# Then we fetch the single global manifest for all the peer share processors.
data "http" "peer_share_processor_global_manifest" {
  url = "https://${var.peer_share_processor_manifest_base_url}/global-manifest.json"
}

locals {
  peer_share_processor_server_identity = jsondecode(data.http.peer_share_processor_global_manifest.body).server-identity
}

module "data_share_processors" {
  for_each = var.ingestors
  source   = "../../modules/data_share_processor"

  environment               = var.environment
  data_share_processor_name = "${var.kubernetes_namespace}-${each.key}"
  ingestor                  = each.key
  use_aws                   = var.use_aws
  aws_region                = var.aws_region
  gcp_region                = var.gcp_region
  gcp_project               = var.gcp_project
  manifest_bucket           = var.manifest_bucket
  kubernetes_namespace      = var.kubernetes_namespace

  ingestor_gcp_service_account_email             = jsondecode(data.http.ingestor_global_manifests[each.key].body).server-identity.gcp-service-account-id
  ingestor_manifest_base_url                     = each.value
  peer_share_processor_aws_account_id            = local.peer_share_processor_server_identity.aws-account-id
  peer_share_processor_gcp_service_account_email = local.peer_share_processor_server_identity.gcp-service-account-email
  peer_share_processor_manifest_base_url         = var.peer_share_processor_manifest_base_url
  remote_bucket_writer_gcp_service_account_email = var.google_service_account_sum_part_bucket_writer_email
  portal_server_manifest_base_url                = var.portal_server_manifest_base_url
  own_manifest_base_url                          = var.manifest_bucket_base_url
  test_peer_environment                          = var.test_peer_environment
  is_first                                       = var.is_first
  aggregation_period                             = var.aggregation_period
  aggregation_grace_period                       = var.aggregation_grace_period
  kms_keyring = var.kms_keyring
}

# Permit the service accounts for all the data share processors to request Oauth
# tokens allowing them to impersonate the sum part bucket writer.
resource "google_service_account_iam_binding" "data_share_processors_to_sum_part_bucket_writer_token_creator" {
  provider           = google-beta
  service_account_id = var.google_service_account_sum_part_bucket_writer_name
  role               = "roles/iam.serviceAccountTokenCreator"
  members            = [for v in module.data_share_processors : "serviceAccount:${v.service_account_email}"]
}

resource "kubernetes_config_map" "manifest_updater_config" {
  metadata {
    name      = "manifest-updater-config"
    namespace = var.kubernetes_namespace
    labels = {
      type : "manifest-updater-config"
    }
  }

  data = {
    "config.json" = jsonencode({
      ingestion_buckets = {
        for v in module.data_share_processors :
        v.ingestor => v.ingestion_bucket_url
      },
      peer_validation_buckets = {
        for v in module.data_share_processors :
        v.ingestor => v.peer_validation_bucket_url
      }
    })
  }
}
