variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "use_aws" {
  type    = bool
  default = false
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

variable "batch_signing_key_expiration" {
  type        = number
  default     = 390
  description = "This value is used to generate batch signing keys with the specified expiration"
}

variable "batch_signing_key_rotation" {
  type        = number
  default     = 300
  description = "This value is used to specify the rotation interval of the batch signing key"
}

variable "packet_encryption_key_expiration" {
  type        = number
  default     = 90
  description = "This value is used to generate packet encryption keys with the specified expiration"
}

variable "packet_encryption_rotation" {
  type        = number
  default     = 50
  description = "This value is used to specify the rotation interval of the packet encryption key"
}

variable "pushgateway" {
  type        = string
  default     = ""
  description = "The location of a pushgateway in host:port form. Set to prometheus-pushgateway.default:9091 to enable metrics"
}

terraform {
  backend "gcs" {}

  required_version = ">= 0.13.3"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 3.16.0"
    }
    google = {
      source = "hashicorp/google"
      # Ensure that this matches the google-beta provider version below.
      version = "~> 3.49.0"
    }
    google-beta = {
      source = "hashicorp/google-beta"
      # Ensure that this matches the non-beta google provider version above.
      version = "~> 3.49.0"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "~> 1.3.2"
    }
    http = {
      source  = "hashicorp/http"
      version = "~> 2.0.0"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 1.13.3"
    }
    null = {
      source  = "hashicorp/null"
      version = "~> 3.0.0"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.0.0"
    }
  }
}

data "terraform_remote_state" "state" {
  backend = "gcs"

  workspace = "${var.environment}-${var.gcp_region}"

  config = {
    bucket = "${var.environment}-${var.gcp_region}-prio-terraform"
  }
}

data "google_client_config" "current" {}

provider "google" {
  # This will use "Application Default Credentials". Run `gcloud auth
  # application-default login` to generate them.
  # https://www.terraform.io/docs/providers/google/guides/provider_reference.html#credentials
  region  = var.gcp_region
  project = var.gcp_project
}

provider "google-beta" {
  # Duplicate settings from the non-beta provider
  region  = var.gcp_region
  project = var.gcp_project
}

# Activate some services which the deployment will require.
resource "google_project_service" "compute" {
  service = "compute.googleapis.com"
}

resource "google_project_service" "container" {
  service = "container.googleapis.com"
}

resource "google_project_service" "kms" {
  service = "cloudkms.googleapis.com"
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

  depends_on = [google_project_service.compute]
}

module "gke" {
  source          = "./modules/gke"
  environment     = var.environment
  resource_prefix = "prio-${var.environment}"
  gcp_region      = var.gcp_region
  gcp_project     = var.gcp_project
  machine_type    = var.machine_type
  network         = google_compute_network.network.self_link
  base_subnet     = local.cluster_subnet_block

  depends_on = [
    google_project_service.compute,
    google_project_service.container,
    google_project_service.kms,
  ]
}

# We fetch the single global manifest for all the peer share processors.
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

# We will receive ingestion batches from multiple ingestion servers for each
# locality. We create a distinct data share processor for each (locality,
# ingestor) pair. e.g., "us-pa-apple" processes data for Pennsylvanians received
# from Apple's server, and "us-az-g-enpa" processes data for Arizonans received
# from Google's server.
# We take the set product of localities x ingestor names to get the config
# values for all the data share processors we need to create.
locals {
  locality_ingestor_pairs = {
    for pair in setproduct(toset(var.localities), keys(var.ingestors)) :
    "${pair[0]}-${pair[1]}" => {
      ingestor                                = pair[1]
      kubernetes_namespace                    = kubernetes_namespace.namespaces[pair[0]].metadata[0].name
      packet_decryption_key_kubernetes_secret = kubernetes_secret.ingestion_packet_decryption_keys[pair[0]].metadata[0].name
      ingestor_manifest_base_url              = var.ingestors[pair[1]]
    }
  }
  peer_share_processor_server_identity = jsondecode(data.http.peer_share_processor_global_manifest.body).server-identity
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

  batch_signing_key_expiration     = var.batch_signing_key_expiration
  batch_signing_key_rotation       = var.batch_signing_key_rotation
  packet_encryption_key_expiration = var.packet_encryption_key_expiration
  packet_encryption_rotation       = var.packet_encryption_rotation
}

module "data_share_processors" {
  for_each                                       = local.locality_ingestor_pairs
  source                                         = "./modules/data_share_processor"
  environment                                    = var.environment
  data_share_processor_name                      = each.key
  ingestor                                       = each.value.ingestor
  use_aws                                        = var.use_aws
  aws_region                                     = var.aws_region
  gcp_region                                     = var.gcp_region
  gcp_project                                    = var.gcp_project
  manifest_bucket                                = module.manifest.bucket
  kubernetes_namespace                           = each.value.kubernetes_namespace
  certificate_domain                             = "${var.environment}.certificates.${var.manifest_domain}"
  ingestor_manifest_base_url                     = each.value.ingestor_manifest_base_url
  packet_decryption_key_kubernetes_secret        = each.value.packet_decryption_key_kubernetes_secret
  peer_share_processor_aws_account_id            = local.peer_share_processor_server_identity.aws-account-id
  peer_share_processor_gcp_service_account_email = local.peer_share_processor_server_identity.gcp-service-account-email
  peer_share_processor_manifest_base_url         = var.peer_share_processor_manifest_base_url
  remote_bucket_writer_gcp_service_account_email = google_service_account.sum_part_bucket_writer.email
  portal_server_manifest_base_url                = var.portal_server_manifest_base_url
  own_manifest_base_url                          = module.manifest.base_url
  test_peer_environment                          = var.test_peer_environment
  is_first                                       = var.is_first
  aggregation_period                             = var.aggregation_period
  aggregation_grace_period                       = var.aggregation_grace_period
  kms_keyring                                    = module.gke.kms_keyring
  pushgateway                                    = var.pushgateway
}

# The portal owns two sum part buckets (one for each data share processor) and
# the one for this data share processor gets configured by the portal operator
# to permit writes from this GCP service account, whose email the portal
# operator discovers in our global manifest.
resource "google_service_account" "sum_part_bucket_writer" {
  account_id   = "prio-${var.environment}-sum-writer"
  display_name = "prio-${var.environment}-sum-part-bucket-writer"
}

# Permit the service accounts for all the data share processors to request Oauth
# tokens allowing them to impersonate the sum part bucket writer.
resource "google_service_account_iam_binding" "data_share_processors_to_sum_part_bucket_writer_token_creator" {
  service_account_id = google_service_account.sum_part_bucket_writer.name
  role               = "roles/iam.serviceAccountTokenCreator"
  members            = [for v in module.data_share_processors : "serviceAccount:${v.service_account_email}"]
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

output "own_manifest_url" {
  value = module.manifest.base_url
}

output "use_test_pha_decryption_key" {
  value = lookup(var.test_peer_environment, "env_without_ingestor", "") == var.environment
}

provider "helm" {
  kubernetes {
    host                   = module.gke.cluster_endpoint
    cluster_ca_certificate = base64decode(module.gke.certificate_authority_data)
    token                  = data.google_client_config.current.access_token
    load_config_file       = false
  }
}

resource "helm_release" "prometheus" {
  name       = "prometheus"
  chart      = "prometheus"
  repository = "https://charts.helm.sh/stable"
}
