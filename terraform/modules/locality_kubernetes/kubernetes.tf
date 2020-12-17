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
      "fqdn" : "${var.kubernetes_namespace}.${var.certificate_domain}",
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

variable "pushgateway" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "workflow_manager_version" {
  type = string
}

variable "facilitator_version" {
  type = string
}

# While we create a distinct data share processor for each (ingestor, locality)
# pair, we only create one packet decryption key for each locality, and use it
# for all ingestors. Since the secret must be in a namespace and accessible
# from all of our data share processors, that means all data share processors
# associated with a given ingestor must be in a single Kubernetes namespace
resource "kubernetes_namespace" "namespaces" {
  metadata {
    name = var.kubernetes_namespace
    annotations = {
      environment = var.environment
    }
  }
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
  kms_keyring                                    = var.kms_keyring
  pushgateway                                    = var.pushgateway
  workflow_manager_version                       = var.workflow_manager_version
  facilitator_version                            = var.facilitator_version
  container_registry                             = var.container_registry
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
