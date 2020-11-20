variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "aws_region" {
  type = string
}

variable "aws_profile" {
  type    = string
  default = "leuswest2"
}

variable "machine_type" {
  type    = string
  default = "e2.small"
}

variable "localities" {
  type = list(string)
}

variable "ingestors" {
  type        = map(string)
  description = "Map of ingestor names to the URL where their global manifest may be found."
}

variable "manifest_domain" {
  type        = string
  description = "Domain (plus optional relative path) to which this environment's global and specific manifests should be uploaded."
}

variable "managed_dns_zone" {
  type = map(string)
}

variable "peer_share_processor_manifest_base_url" {
  type = string
}

variable "portal_server_manifest_base_url" {
  type = string
}

variable "test_peer_environment" {
  type        = map(string)
  default     = {}
  description = <<DESCRIPTION
Describes a pair of data share processor environments set up to test against
each other. One environment, named in "env_with_ingestor", hosts a fake
ingestion server. The other, named in "env_without_ingestor", does not. This
variable should not be specified in production deployments.
DESCRIPTION
}

variable "is_first" {
  type        = bool
  default     = false
  description = "Whether the data share processors created by this environment are \"first\" or \"PHA servers\""
}

variable "aggregation_period" {
  type        = string
  default     = "3h"
  description = <<DESCRIPTION
Aggregation period used by workflow manager. The value should be a string
parseable by Go's time.ParseDuration.
DESCRIPTION
}

variable "aggregation_grace_period" {
  type        = string
  default     = "1h"
  description = <<DESCRIPTION
Aggregation grace period used by workflow manager. The value should be a string
parseable by Go's time.ParseDuration.
DESCRIPTION
}

terraform {
  backend "gcs" {}

  required_version = ">= 0.13.3"
}

data "terraform_remote_state" "state" {
  backend = "gcs"

  workspace = "${var.environment}-${var.gcp_region}"

  config = {
    bucket = "${var.environment}-${var.gcp_region}-prio-terraform"
  }
}

data "google_client_config" "current" {}

provider "google-beta" {
  # We use the google-beta provider so that we can use configuration fields that
  # aren't in the GA google provider. Google resources must explicitly opt into
  # this provider with `provider = google-beta` or they will not inherit values
  # appropriately.
  # https://www.terraform.io/docs/providers/google/guides/provider_versions.html
  # This will use "Application Default Credentials". Run `gcloud auth
  # application-default login` to generate them.
  # https://www.terraform.io/docs/providers/google/guides/provider_reference.html#credentials
  region  = var.gcp_region
  project = var.gcp_project
}

provider "aws" {
  # aws_s3_bucket resources will be created in the region specified in this
  # provider.
  # https://github.com/hashicorp/terraform/issues/12512
  region  = var.aws_region
  profile = var.aws_profile
}

provider "kubernetes" {
  host                   = module.gke.cluster_endpoint
  cluster_ca_certificate = base64decode(module.gke.certificate_authority_data)
  token                  = data.google_client_config.current.access_token
  load_config_file       = false
}

module "manifest" {
  source                                = "./modules/manifest"
  environment                           = var.environment
  gcp_region                            = var.gcp_region
  managed_dns_zone                      = var.managed_dns_zone
  sum_part_bucket_service_account_email = google_service_account.sum_part_bucket_writer.email
}

module "gke" {
  source          = "./modules/gke"
  environment     = var.environment
  resource_prefix = "prio-${var.environment}"
  gcp_region      = var.gcp_region
  gcp_project     = var.gcp_project
  machine_type    = var.machine_type
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

# While we create a distinct data share processor for each (ingestor, locality)
# pair, we only create one packet decryption key for each locality, and use it
# for all ingestors. Since the secret must be in a namespace and accessible
# from all of our data share processors, that means all data share processors
# associated with a given ingestor must be in a single Kubernetes namespace,
# which we create here and pass into the data share processor module.
resource "kubernetes_namespace" "namespaces" {
  for_each = toset(var.localities)
  metadata {
    name = each.key
    annotations = {
      environment = var.environment
    }
  }
}

resource "kubernetes_secret" "ingestion_packet_decryption_keys" {
  for_each = toset(var.localities)
  metadata {
    name      = "${var.environment}-${each.key}-ingestion-packet-decryption-key"
    namespace = kubernetes_namespace.namespaces[each.key].metadata[0].name
  }

  data = {
    # See comment on batch_signing_key, in modules/kubernetes/kubernetes.tf,
    # about the initial value and the lifecycle block here.
    secret_key = "not-a-real-key"
  }

  lifecycle {
    ignore_changes = [
      data["secret_key"]
    ]
  }
}

# Now, we take the set product of localities x ingestor names to
# get the config values for all the data share processors we need to create.
locals {
  locality_ingestor_pairs = {
    for pair in setproduct(toset(var.localities), keys(var.ingestors)) :
    "${pair[0]}-${pair[1]}" => {
      ingestor                                = pair[1]
      kubernetes_namespace                    = kubernetes_namespace.namespaces[pair[0]].metadata[0].name
      packet_decryption_key_kubernetes_secret = kubernetes_secret.ingestion_packet_decryption_keys[pair[0]].metadata[0].name
      ingestor_aws_role_arn                   = lookup(jsondecode(data.http.ingestor_global_manifests[pair[1]].body).server-identity, "aws-iam-entity", "")
      ingestor_gcp_service_account_id         = lookup(jsondecode(data.http.ingestor_global_manifests[pair[1]].body).server-identity, "google-service-account", "")
      ingestor_gcp_service_account_email      = lookup(jsondecode(data.http.ingestor_global_manifests[pair[1]].body).server-identity, "gcp-service-account-email", "")
      ingestor_manifest_base_url              = var.ingestors[pair[1]]
    }
  }
}

# Call the locality_kubernetes module per
# each locality/namespace
module "locality_kubernetes" {
  for_each             = kubernetes_namespace.namespaces
  source               = "./modules/locality_kubernetes"
  environment          = var.environment
  gcp_project          = var.gcp_project
  manifest_bucket      = module.manifest.bucket
  kubernetes_namespace = each.value.metadata[0].name
  ingestors            = keys(var.ingestors)
  depends_on           = [module.manifest]
}

module "data_share_processors" {
  for_each                                = local.locality_ingestor_pairs
  source                                  = "./modules/data_share_processor"
  environment                             = var.environment
  data_share_processor_name               = each.key
  ingestor                                = each.value.ingestor
  gcp_region                              = var.gcp_region
  gcp_project                             = var.gcp_project
  manifest_bucket                         = module.manifest.bucket
  kubernetes_namespace                    = each.value.kubernetes_namespace
  certificate_domain                      = "${var.environment}.certificates.${var.manifest_domain}"
  ingestor_aws_role_arn                   = each.value.ingestor_aws_role_arn
  ingestor_gcp_service_account_id         = each.value.ingestor_gcp_service_account_id
  ingestor_gcp_service_account_email      = each.value.ingestor_gcp_service_account_email
  ingestor_manifest_base_url              = each.value.ingestor_manifest_base_url
  packet_decryption_key_kubernetes_secret = each.value.packet_decryption_key_kubernetes_secret
  peer_share_processor_aws_account_id     = jsondecode(data.http.peer_share_processor_global_manifest.body).server-identity.aws-account-id
  peer_share_processor_manifest_base_url  = var.peer_share_processor_manifest_base_url
  sum_part_bucket_service_account_email   = google_service_account.sum_part_bucket_writer.email
  portal_server_manifest_base_url         = var.portal_server_manifest_base_url
  own_manifest_base_url                   = module.manifest.base_url
  test_peer_environment                   = var.test_peer_environment
  is_first                                = var.is_first
  aggregation_period                      = var.aggregation_period
  aggregation_grace_period                = var.aggregation_grace_period

  depends_on = [module.gke]
}

# The portal owns two sum part buckets (one for each data share processor) and
# the one for this data share processor gets configured by the portal operator
# to permit writes from this GCP service account, whose email the portal
# operator discovers in our global manifest.
resource "google_service_account" "sum_part_bucket_writer" {
  provider     = google-beta
  account_id   = "prio-${var.environment}-sum-writer"
  display_name = "prio-${var.environment}-sum-part-bucket-writer"
}

# Permit the service accounts for all the data share processors to request Oauth
# tokens allowing them to impersonate the sum part bucket writer.
resource "google_service_account_iam_binding" "data_share_processors_to_sum_part_bucket_writer_token_creator" {
  provider           = google-beta
  service_account_id = google_service_account.sum_part_bucket_writer.name
  role               = "roles/iam.serviceAccountTokenCreator"
  members            = [for v in module.data_share_processors : v.service_account_email]
}

module "fake_server_resources" {
  count                        = lookup(var.test_peer_environment, "env_with_ingestor", "") == "" ? 0 : 1
  source                       = "./modules/fake_server_resources"
  manifest_bucket              = module.manifest.bucket
  gcp_region                   = var.gcp_region
  environment                  = var.environment
  sum_part_bucket_writer_email = google_service_account.sum_part_bucket_writer.email
  ingestors                    = var.ingestors
}

output "manifest_bucket" {
  value = module.manifest.bucket
}

output "gke_kubeconfig" {
  value = "Run this command to update your kubectl config: gcloud container clusters get-credentials ${module.gke.cluster_name} --region ${var.gcp_region} --project ${var.gcp_project}"
}

output "specific_manifests" {
  value = { for v in module.data_share_processors : v.data_share_processor_name => {
    kubernetes-namespace = v.kubernetes_namespace
    certificate-fqdn     = v.certificate_fqdn
    specific-manifest    = v.specific_manifest
    }
  }
}

output "use_test_pha_decryption_key" {
  value = lookup(var.test_peer_environment, "env_without_ingestor", "") == var.environment
}
