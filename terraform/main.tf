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

variable "localities" {
  type = list(string)
}

variable "ingestors" {
  type = map(object({
    manifest_base_url = string
    localities = map(object({
      intake_worker_count                    = number
      aggregate_worker_count                 = number
      peer_share_processor_manifest_base_url = string
      portal_server_manifest_base_url        = string
    }))
  }))
  description = "Map of ingestor names to per-ingestor configuration."
}

variable "manifest_domain" {
  type        = string
  description = "Domain (plus optional relative path) to which this environment's global and specific manifests should be uploaded."
}

variable "managed_dns_zone" {
  type = map(string)
}

variable "test_peer_environment" {
  type = object({
    env_with_ingestor            = string
    env_without_ingestor         = string
    localities_with_sample_maker = list(string)
  })
  default = {
    env_with_ingestor            = ""
    env_without_ingestor         = ""
    localities_with_sample_maker = []
  }
  description = <<DESCRIPTION
Describes a pair of data share processor environments set up to test against
each other. One environment, named in "env_with_ingestor", hosts a fake
ingestion servers, but only for the localities enumerated in
"localities_with_sample_makers", which should be a subset of the ones in
"localities". The other environment, named in "env_without_ingestor", has no
fake ingestion servers. This variable should not be specified in production
deployments.
DESCRIPTION
}

variable "is_first" {
  type        = bool
  default     = false
  description = "Whether the data share processors created by this environment are \"first\" or \"PHA servers\""
}

variable "intake_max_age" {
  type        = string
  default     = "6h"
  description = <<DESCRIPTION
Maximum age of ingestion batches for workflow-manager to schedule intake tasks
for. The value should be a string parseable by Go's time.ParseDuration.
DESCRIPTION
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

variable "container_registry" {
  type    = string
  default = "letsencrypt"
}

variable "workflow_manager_image" {
  type    = string
  default = "prio-workflow-manager"
}

variable "workflow_manager_version" {
  type    = string
  default = "latest"
}

variable "facilitator_image" {
  type    = string
  default = "prio-facilitator"
}

variable "facilitator_version" {
  type    = string
  default = "latest"
}

variable "prometheus_server_persistent_disk_size_gb" {
  type = number
  # This is quite high, but it's the minimum for GCE regional disks
  default = 200
}

variable "victorops_routing_key" {
  type        = string
  description = "VictorOps/Splunk OnCall routing key for prometheus-alertmanager"
  default     = "bogus-routing-key"
}

variable "cluster_settings" {
  type = object({
    initial_node_count = number
    min_node_count     = number
    max_node_count     = number
    machine_type       = string
  })
}

terraform {
  backend "gcs" {}

  required_version = ">= 0.14.4"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 3.49.0"
    }
    google = {
      source = "hashicorp/google"
      # Ensure that this matches the google-beta provider version below.
      version = "~> 3.72.0"
    }
    google-beta = {
      source = "hashicorp/google-beta"
      # Ensure that this matches the non-beta google provider version above.
      version = "~> 3.72.0"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "~> 2.2.0"
    }
    http = {
      source  = "hashicorp/http"
      version = "~> 2.1.0"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.3.2"
    }
    null = {
      source  = "hashicorp/null"
      version = "~> 3.1.0"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.1.0"
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
  source           = "./modules/gke"
  environment      = var.environment
  resource_prefix  = "prio-${var.environment}"
  gcp_region       = var.gcp_region
  gcp_project      = var.gcp_project
  cluster_settings = var.cluster_settings
  network          = google_compute_network.network.self_link
  base_subnet      = local.cluster_subnet_block

  depends_on = [
    google_project_service.compute,
    google_project_service.container,
    google_project_service.kms,
  ]
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
      locality                                = pair[0]
      kubernetes_namespace                    = kubernetes_namespace.namespaces[pair[0]].metadata[0].name
      packet_decryption_key_kubernetes_secret = kubernetes_secret.ingestion_packet_decryption_keys[pair[0]].metadata[0].name
      ingestor_manifest_base_url              = var.ingestors[pair[1]].manifest_base_url
      intake_worker_count                     = var.ingestors[pair[1]].localities[pair[0]].intake_worker_count
      aggregate_worker_count                  = var.ingestors[pair[1]].localities[pair[0]].aggregate_worker_count
      peer_share_processor_manifest_base_url  = var.ingestors[pair[1]].localities[pair[0]].peer_share_processor_manifest_base_url
      portal_server_manifest_base_url         = var.ingestors[pair[1]].localities[pair[0]].portal_server_manifest_base_url
    }
  }
  # For now, we only support advertising manifests in GCS but in #655 we will
  # add support for S3
  manifest = {
    bucket      = module.manifest.bucket
    base_url    = module.manifest.base_url
    aws_region  = ""
    aws_profile = ""
  }
}

locals {
  // Does this series of deployment have a fake ingestor (integration-tester)
  deployment_has_ingestor = lookup(var.test_peer_environment, "env_with_ingestor", "") == "" ? false : true
  // Does this specific environment home the ingestor
  is_env_with_ingestor = local.deployment_has_ingestor && lookup(var.test_peer_environment, "env_with_ingestor", "") == var.environment ? true : false
}

# Call the locality_kubernetes module per
# each locality/namespace
module "locality_kubernetes" {
  for_each             = kubernetes_namespace.namespaces
  source               = "./modules/locality_kubernetes"
  environment          = var.environment
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
  kubernetes_namespace                           = each.value.kubernetes_namespace
  certificate_domain                             = "${var.environment}.certificates.${var.manifest_domain}"
  ingestor_manifest_base_url                     = each.value.ingestor_manifest_base_url
  packet_decryption_key_kubernetes_secret        = each.value.packet_decryption_key_kubernetes_secret
  peer_share_processor_manifest_base_url         = each.value.peer_share_processor_manifest_base_url
  remote_bucket_writer_gcp_service_account_email = google_service_account.sum_part_bucket_writer.email
  portal_server_manifest_base_url                = each.value.portal_server_manifest_base_url
  own_manifest_base_url                          = module.manifest.base_url
  is_first                                       = var.is_first
  intake_max_age                                 = var.intake_max_age
  aggregation_period                             = var.aggregation_period
  aggregation_grace_period                       = var.aggregation_grace_period
  kms_keyring                                    = module.gke.kms_keyring
  pushgateway                                    = var.pushgateway
  workflow_manager_image                         = var.workflow_manager_image
  workflow_manager_version                       = var.workflow_manager_version
  facilitator_image                              = var.facilitator_image
  facilitator_version                            = var.facilitator_version
  container_registry                             = var.container_registry
  intake_worker_count                            = each.value.intake_worker_count
  aggregate_worker_count                         = each.value.aggregate_worker_count
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
  count                 = local.is_env_with_ingestor ? 1 : 0
  source                = "./modules/fake_server_resources"
  manifest_bucket       = module.manifest.bucket
  gcp_region            = var.gcp_region
  environment           = var.environment
  ingestor_pairs        = local.locality_ingestor_pairs
  own_manifest_base_url = module.manifest.base_url
  pushgateway           = var.pushgateway
  container_registry    = var.container_registry
  facilitator_image     = var.facilitator_image
  facilitator_version   = var.facilitator_version

  depends_on = [module.gke]
}

module "portal_server_resources" {
  count                        = local.deployment_has_ingestor ? 1 : 0
  source                       = "./modules/portal_server_resources"
  manifest_bucket              = module.manifest.bucket
  gcp_region                   = var.gcp_region
  environment                  = var.environment
  sum_part_bucket_writer_email = google_service_account.sum_part_bucket_writer.email

  depends_on = [module.gke]
}

module "monitoring" {
  source                 = "./modules/monitoring"
  environment            = var.environment
  gcp_region             = var.gcp_region
  cluster_endpoint       = module.gke.cluster_endpoint
  cluster_ca_certificate = base64decode(module.gke.certificate_authority_data)
  victorops_routing_key  = var.victorops_routing_key
  aggregation_period     = var.aggregation_period

  prometheus_server_persistent_disk_size_gb = var.prometheus_server_persistent_disk_size_gb
}

output "manifest_bucket" {
  value = {
    bucket      = local.manifest.bucket
    aws_region  = local.manifest.aws_region
    aws_profile = local.manifest.aws_profile
  }
}

output "gke_kubeconfig" {
  value = "Run this command to update your kubectl config: gcloud container clusters get-credentials ${module.gke.cluster_name} --region ${var.gcp_region} --project ${var.gcp_project}"
}

output "specific_manifests" {
  value = { for v in module.data_share_processors : v.data_share_processor_name => {
    ingestor-name        = v.ingestor_name
    kubernetes-namespace = v.kubernetes_namespace
    certificate-fqdn     = v.certificate_fqdn
    specific-manifest    = v.specific_manifest
    }
  }
}

output "singleton_ingestor" {
  value = local.is_env_with_ingestor ? {
    aws_iam_entity              = module.fake_server_resources[0].aws_iam_entity
    gcp_service_account_id      = module.fake_server_resources[0].gcp_service_account_id
    gcp_service_account_email   = module.fake_server_resources[0].gcp_service_account_email
    tester_kubernetes_namespace = module.fake_server_resources[0].test_kubernetes_namespace
    batch_signing_key_name      = module.fake_server_resources[0].batch_signing_key_name
  } : {}
}
output "own_manifest_base_url" {
  value = local.manifest.base_url
}

output "use_test_pha_decryption_key" {
  value = lookup(var.test_peer_environment, "env_without_ingestor", "") == var.environment
}

output "has_test_environment" {
  value = length(module.fake_server_resources) != 0
}
