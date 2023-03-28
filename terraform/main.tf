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

variable "pure_gcp" {
  type        = bool
  default     = false
  description = <<DESCRIPTION
A data share processor that runs on GCP (i.e., use_aws=false) will still manage
some resources in AWS (IAM roles) _unless_ this variable is set to true.
DESCRIPTION
}

variable "aws_region" {
  type = string
}

variable "allowed_aws_account_ids" {
  type    = list(string)
  default = null
}

variable "localities" {
  type = list(string)
}

variable "ingestors" {
  type = map(object({
    manifest_base_url = string
    localities = map(object({
      intake_worker_count     = optional(number) # Deprecated: set {min,max}_intake_worker_count instead.
      min_intake_worker_count = optional(number)
      max_intake_worker_count = optional(number)

      aggregate_worker_count     = optional(number) # Deprecated: set {min,max}_aggregate_worker_count instead.
      min_aggregate_worker_count = optional(number)
      max_aggregate_worker_count = optional(number)

      peer_share_processor_manifest_base_url = optional(string)
      portal_server_manifest_base_url        = optional(string)
      aggregation_period                     = optional(string)
      aggregation_grace_period               = optional(string)

      pure_gcp = optional(bool)
    }))
  }))
  description = <<DESCRIPTION
Map of ingestor names to per-ingestor configuration.
peer_share_processor_manifest_base_url is optional and overrides
default_peer_share_processor_manifest_base_url for the locality.
portal_server_manifest_base_url is optional and overrides
default_portal_server_manifest_base_url for the locality.
aggregation_period and aggregation_grace_period values are optional and override
default_aggregation_period and default_aggregation_grace_period, respectively,
for the locality. The values should be strings parseable by Go's
time.ParseDuration.
DESCRIPTION
}

variable "manifest_domain" {
  type        = string
  description = "Domain (plus optional relative path) to which this environment's global and specific manifests should be uploaded."
}

variable "managed_dns_zone" {
  type = object({
    name        = string
    gcp_project = string
  })
  default = {
    name        = ""
    gcp_project = ""
  }
}

variable "test_peer_environment" {
  type = object({
    env_with_ingestor    = string
    env_without_ingestor = string
  })
  default = {
    env_with_ingestor    = ""
    env_without_ingestor = ""
  }
  description = <<DESCRIPTION
Describes a pair of data share processor environments set up to test against
each other. One environment, named in "env_with_ingestor", hosts fake
ingestion servers writing ingestion batches to both environments, as well as
validators that check that the eventual sums match the data in the ingestion
batches. The other environment, named in "env_without_ingestor", has no fake
ingestion servers or validators.

This variable should not be specified in production deployments.
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

variable "default_aggregation_period" {
  type        = string
  default     = "3h"
  description = <<DESCRIPTION
Aggregation period used by workflow manager if none is provided by the locality
configuration. The value should be a string parseable by Go's
time.ParseDuration.
DESCRIPTION
}

variable "default_aggregation_grace_period" {
  type        = string
  default     = "1h"
  description = <<DESCRIPTION
Aggregation grace period used by workflow manager if none is provided by the locality
configuration. The value should be a string parseable by Go's
time.ParseDuration.
DESCRIPTION
}

variable "default_peer_share_processor_manifest_base_url" {
  type        = string
  description = <<DESCRIPTION
Base URL relative to which the peer share processor's manifests can be found, if
none is provided by the locality configuration.
DESCRIPTION
}

variable "default_portal_server_manifest_base_url" {
  type        = string
  description = <<DESCRIPTION
Base URL relative to which the portal server's manifests can be found, if none
is provided by the locality configuration.
DESCRIPTION
}

variable "pushgateway" {
  type        = string
  default     = "prometheus-pushgateway.monitoring:9091"
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

variable "key_rotator_image" {
  type    = string
  default = "prio-key-rotator"
}

variable "key_rotator_version" {
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
  default     = "bogus-routing-key"
  description = "VictorOps/Splunk OnCall routing key for prometheus-alertmanager"
}

variable "batch_signing_key_rotation_policy" {
  type = object({
    create_min_age   = string
    primary_min_age  = string
    delete_min_age   = string
    delete_min_count = number
  })
  default = {
    create_min_age   = "6480h" // 6480 hours = 9 months (w/ 30-day months, 24-hour days)
    primary_min_age  = "168h"  // 168 hours = 1 week (w/ 24-hour days)
    delete_min_age   = "9360h" // 9360 hours = 13 months (w/ 30-day months, 24-hour days)
    delete_min_count = 2
  }
}

variable "packet_encryption_key_rotation_policy" {
  type = object({
    create_min_age   = string
    primary_min_age  = string
    delete_min_age   = string
    delete_min_count = number
  })
  default = {
    create_min_age   = "6480h" // 6480 hours = 9 months (w/ 30-day months, 24-hour days)
    primary_min_age  = "23h"
    delete_min_age   = "9360h" // 9360 hours = 13 months (w/ 30-day months, 24-hour days)
    delete_min_count = 2
  }
}

variable "key_rotator_schedule" {
  type    = string
  default = "0 20 * * MON-THU" # 8 PM (UTC) daily, Monday thru Thursday
}

variable "enable_key_rotation_localities" {
  type    = list(string)
  default = []
}

variable "prometheus_helm_chart_version" {
  type = string
  # The default is the empty string, which uses the latest available version at
  # the time of apply
  default = ""
}

variable "grafana_helm_chart_version" {
  type    = string
  default = ""
}

variable "cloudwatch_exporter_helm_chart_version" {
  type    = string
  default = ""
}

variable "stackdriver_exporter_helm_chart_version" {
  type    = string
  default = ""
}

variable "single_object_validation_batch_localities" {
  type        = list(string)
  default     = []
  description = <<DESCRIPTION
A set of localities where single-object validation batches are generated.
(Other localities use the "old" three-object format.) The special value "*"
indicates that all localities should generate single-object validation batches.
DESCRIPTION
}

variable "state_bucket" {
  type        = string
  description = <<DESCRIPTION
The name of the GCS bucket that Terraform's state is stored in, for access
to outputs from the cluster_bootstrap stage.
DESCRIPTION
}

variable "cluster_settings" {
  type = object({
    initial_node_count             = number
    min_node_count                 = number
    max_node_count                 = number
    gcp_machine_type               = string
    aws_machine_types              = list(string)
    eks_cluster_version            = optional(string)
    eks_vpc_cni_addon_version      = optional(string)
    eks_ebs_csi_addon_version      = optional(string)
    eks_cluster_autoscaler_version = optional(string)
  })
  description = <<DESCRIPTION
Settings for the Kubernetes cluster.

  - `initial_node_count` is the initial number of worker nodes to provision
  - `min_node_count` is the minimum number of worker nodes
  - `max_node_count` is the maximum number of worker nodes
  - `gcp_machine_type` is the type and size of VM to use as worker nodes in GKE
    clusters
  - `aws_machine_types` is the types and sizes of VMs to use as worker nodes in
    EKS clusters
  - `eks_cluster_version` is the Amazon EKS Kubernetes version to use. See
    https://docs.aws.amazon.com/eks/latest/userguide/kubernetes-versions.html
    for available versions. Only used in EKS clusters.
  - `eks_vpc_cni_addon_version` is the Amazon VPC CNI add-on version to use. See
    https://docs.aws.amazon.com/eks/latest/userguide/managing-vpc-cni.html for
    available versions. Only used in EKS clusters.
  - `eks_ebs_csi_addon_version` is the Amazon EBS CSI driver add-on version to
    use. See https://github.com/kubernetes-sigs/aws-ebs-csi-driver/releases for
    available versions. Only used in EKS clusters.
  - `eks_cluster_autoscaler_version` is the version of the Kubernetes cluster
    autoscaler to use. The version must match the minor version of Kubernetes.
    See https://github.com/kubernetes/autoscaler/releases for available
    versions. Only used in EKS clusters.

Note that on AWS, there will always be at least three worker nodes in an "on
demand" node pool, and the `{min,max}_node_count` values only govern the size of
an additional "spot" node pool.
DESCRIPTION
}

variable "role_permissions_boundary_policy_arn" {
  type        = string
  default     = null
  description = <<-DESCRIPTION
  Optional AWS IAM policy ARN, to be used as the permissions boundary policy
  on any IAM roles created for a non-pure GCP environment.
  DESCRIPTION
}

variable "enable_heap_profiles" {
  type        = bool
  default     = false
  description = <<-DESCRIPTION
  If true, set up an NFS server, and have Go language CronJob pods save heap
  profiles to an NFS volume.
  DESCRIPTION
}

terraform {
  backend "gcs" {}

  required_version = ">= 1.3.1"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "4.60.0"
    }
    google = {
      source = "hashicorp/google"
      # Keep this version in sync with provider google-beta
      version = "4.59.0"
    }
    google-beta = {
      source = "hashicorp/google-beta"
      # Keep this version in sync with provider google
      version = "4.58.0"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "2.9.0"
    }
    http = {
      source  = "hashicorp/http"
      version = "3.2.1"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "2.19.0"
    }
    null = {
      source  = "hashicorp/null"
      version = "3.2.1"
    }
    random = {
      source  = "hashicorp/random"
      version = "3.4.3"
    }
    kubectl = {
      source  = "gavinbunney/kubectl"
      version = "1.14.0"
    }
    external = {
      source  = "hashicorp/external"
      version = "2.3.1"
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

data "google_project" "current" {}
data "google_client_config" "current" {}
data "aws_caller_identity" "current" {}

provider "google" {
  # This will use "Application Default Credentials". Run `gcloud auth
  # application-default login` to generate them.
  # https://www.terraform.io/docs/providers/google/guides/provider_reference.html#credentials
  region  = var.gcp_region
  project = var.gcp_project
}

provider "google-beta" {
  region  = var.gcp_region
  project = var.gcp_project
}

# AWS provider credentials come from environment variables set by the `aws-mfa`
# script
provider "aws" {
  # aws_s3_bucket resources will be created in the region specified in this
  # provider.
  # https://github.com/hashicorp/terraform/issues/12512
  region              = var.aws_region
  allowed_account_ids = var.allowed_aws_account_ids

  default_tags {
    tags = {
      "prio-env" = var.environment
    }
  }
}

provider "kubernetes" {
  host                   = local.kubernetes_cluster.endpoint
  cluster_ca_certificate = base64decode(local.kubernetes_cluster.certificate_authority_data)
  token                  = local.kubernetes_cluster.token
}

provider "helm" {
  kubernetes {
    host                   = local.kubernetes_cluster.endpoint
    cluster_ca_certificate = base64decode(local.kubernetes_cluster.certificate_authority_data)
    token                  = local.kubernetes_cluster.token
  }
}

# We use provider kubectl to manage Kubernetes manifests, as a workaround for an
# issue in hashicorp/kubernetes:
# https://github.com/hashicorp/terraform-provider-kubernetes/issues/1380
provider "kubectl" {
  host                   = local.kubernetes_cluster.endpoint
  cluster_ca_certificate = base64decode(local.kubernetes_cluster.certificate_authority_data)
  token                  = local.kubernetes_cluster.token
}

module "manifest_gcp" {
  source                  = "./modules/manifest_gcp"
  count                   = var.use_aws ? 0 : 1
  environment             = var.environment
  gcp_region              = var.gcp_region
  global_manifest_content = local.global_manifest
  managed_dns_zone        = var.managed_dns_zone
}

module "manifest_aws" {
  source                  = "./modules/manifest_aws"
  count                   = var.use_aws ? 1 : 0
  environment             = var.environment
  global_manifest_content = local.global_manifest
}

module "eks" {
  source           = "./modules/eks"
  count            = var.use_aws ? 1 : 0
  environment      = var.environment
  aws_region       = var.aws_region
  cluster_settings = var.cluster_settings
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
      data
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
      min_intake_worker_count = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].min_intake_worker_count,
        var.ingestors[pair[1]].localities[pair[0]].intake_worker_count
      )
      max_intake_worker_count = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].max_intake_worker_count,
        var.ingestors[pair[1]].localities[pair[0]].intake_worker_count
      )
      min_aggregate_worker_count = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].min_aggregate_worker_count,
        var.ingestors[pair[1]].localities[pair[0]].aggregate_worker_count
      )
      max_aggregate_worker_count = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].max_aggregate_worker_count,
        var.ingestors[pair[1]].localities[pair[0]].aggregate_worker_count
      )
      peer_share_processor_manifest_base_url = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].peer_share_processor_manifest_base_url,
        var.default_peer_share_processor_manifest_base_url
      )
      portal_server_manifest_base_url = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].portal_server_manifest_base_url,
        var.default_portal_server_manifest_base_url
      )
      aggregation_period = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].aggregation_period,
        var.default_aggregation_period
      )
      aggregation_grace_period = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].aggregation_grace_period,
        var.default_aggregation_grace_period
      )
      pure_gcp = coalesce(
        var.ingestors[pair[1]].localities[pair[0]].pure_gcp,
        var.pure_gcp
      )
    }
  }
  # Are we in a paired env deploy that uses a test ingestor?
  deployment_has_ingestor = lookup(var.test_peer_environment, "env_with_ingestor", "") == "" ? false : true
  # Does this specific environment home the ingestor?
  is_env_with_ingestor = local.deployment_has_ingestor && lookup(var.test_peer_environment, "env_with_ingestor", "") == var.environment ? true : false

  global_manifest = jsonencode({
    format = 1
    server-identity = {
      aws-account-id            = tonumber(data.aws_caller_identity.current.account_id)
      gcp-service-account-id    = var.use_aws ? null : tostring(google_service_account.sum_part_bucket_writer.unique_id)
      gcp-service-account-email = google_service_account.sum_part_bucket_writer.email
    }
  })

  manifest = var.use_aws ? {
    bucket         = module.manifest_aws[0].bucket
    bucket_url     = module.manifest_aws[0].bucket_url
    base_url       = module.manifest_aws[0].base_url
    aws_bucket_arn = module.manifest_aws[0].bucket_arn
    aws_region     = var.aws_region
    } : {
    bucket         = module.manifest_gcp[0].bucket
    bucket_url     = module.manifest_gcp[0].bucket_url
    base_url       = module.manifest_gcp[0].base_url
    aws_bucket_arn = ""
    aws_region     = ""
  }

  kubernetes_cluster = var.use_aws ? {
    name                       = module.eks[0].cluster_name
    endpoint                   = module.eks[0].cluster_endpoint
    certificate_authority_data = module.eks[0].certificate_authority_data
    token                      = module.eks[0].token
    } : {
    name                       = data.google_container_cluster.cluster[0].name
    endpoint                   = "https://${data.google_container_cluster.cluster[0].endpoint}"
    certificate_authority_data = data.google_container_cluster.cluster[0].master_auth.0.cluster_ca_certificate
    token                      = data.google_client_config.current.access_token
  }
}

data "google_container_cluster" "cluster" {
  count = var.use_aws ? 0 : 1

  name     = "prio-${var.environment}-cluster"
  location = var.gcp_region
}

data "terraform_remote_state" "cluster_bootstrap" {
  backend = "gcs"
  config = {
    bucket = var.state_bucket
    prefix = "cluster-bootstrap"
  }
}

resource "google_project_iam_custom_role" "gcp_secret_writer" {
  count = var.use_aws ? 0 : 1

  role_id     = "secret_manager_secret_writer"
  title       = "Secret Writer"
  description = "Allowed to write secrets & secret versions."
  permissions = [
    "secretmanager.secrets.create",
    "secretmanager.versions.add",
  ]
}

module "kubernetes_locality" {
  for_each = toset(var.localities)
  source   = "./modules/kubernetes_locality"

  environment                           = var.environment
  container_registry                    = var.container_registry
  key_rotator_image                     = var.key_rotator_image
  key_rotator_version                   = var.key_rotator_version
  use_aws                               = var.use_aws
  gcp_project                           = var.gcp_project
  gcp_secret_writer_role_id             = var.use_aws ? "" : google_project_iam_custom_role.gcp_secret_writer[0].id
  eks_oidc_provider                     = var.use_aws ? module.eks[0].oidc_provider : { url = "", arn = "" }
  manifest_bucket                       = local.manifest
  kubernetes_namespace                  = kubernetes_namespace.namespaces[each.key].metadata[0].name
  locality                              = each.key
  ingestors                             = keys(var.ingestors)
  certificate_fqdn                      = "${kubernetes_namespace.namespaces[each.key].metadata[0].name}.${var.environment}.certificates.${var.manifest_domain}"
  pushgateway                           = var.pushgateway
  batch_signing_key_rotation_policy     = var.batch_signing_key_rotation_policy
  packet_encryption_key_rotation_policy = var.packet_encryption_key_rotation_policy
  enable_key_rotator_localities         = toset(var.enable_key_rotation_localities)
  key_rotator_schedule                  = var.key_rotator_schedule
  specific_manifest_templates           = { for v in module.data_share_processors : v.data_share_processor_name => v.specific_manifest }
  enable_heap_profiles                  = var.enable_heap_profiles
  profile_nfs_server                    = length(module.nfs_server) > 0 ? module.nfs_server[0].server : null
}

module "data_share_processors" {
  for_each                                       = local.locality_ingestor_pairs
  source                                         = "./modules/data_share_processor"
  environment                                    = var.environment
  data_share_processor_name                      = each.key
  ingestor                                       = each.value.ingestor
  use_aws                                        = var.use_aws
  pure_gcp                                       = each.value.pure_gcp
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
  is_first                                       = var.is_first
  intake_max_age                                 = var.intake_max_age
  aggregation_period                             = each.value.aggregation_period
  aggregation_grace_period                       = each.value.aggregation_grace_period
  kms_keyring                                    = data.terraform_remote_state.cluster_bootstrap.outputs.google_kms_key_ring_id
  pushgateway                                    = var.pushgateway
  workflow_manager_image                         = var.workflow_manager_image
  workflow_manager_version                       = var.workflow_manager_version
  facilitator_image                              = var.facilitator_image
  facilitator_version                            = var.facilitator_version
  container_registry                             = var.container_registry
  min_intake_worker_count                        = each.value.min_intake_worker_count
  max_intake_worker_count                        = each.value.max_intake_worker_count
  min_aggregate_worker_count                     = each.value.min_aggregate_worker_count
  max_aggregate_worker_count                     = each.value.max_aggregate_worker_count
  eks_oidc_provider                              = var.use_aws ? module.eks[0].oidc_provider : { url = "", arn = "" }
  gcp_workload_identity_pool_provider            = local.gcp_workload_identity_pool_provider
  single_object_validation_batch_localities      = toset(var.single_object_validation_batch_localities)
  role_permissions_boundary_policy_arn           = var.role_permissions_boundary_policy_arn
  enable_heap_profiles                           = var.enable_heap_profiles
  profile_nfs_server                             = length(module.nfs_server) > 0 ? module.nfs_server[0].server : null
}

# The portal owns two sum part buckets (one for each data share processor) and
# the one for this data share processor gets configured by the portal operator
# to permit writes from this GCP service account, whose email the portal
# operator discovers in our global manifest. We use a GCP SA to write sum parts
# even if our data share processor runs on AWS.
resource "google_service_account" "sum_part_bucket_writer" {
  account_id   = "prio-${var.environment}-sum-writer"
  display_name = "prio-${var.environment}-sum-part-bucket-writer"
}

# If running in GCP, permit the service accounts for all the data share
# processors to request access and identity tokens allowing them to impersonate
# the sum part bucket writer.
resource "google_service_account_iam_member" "data_share_processors_to_sum_part_bucket_writer_token_creator" {
  for_each           = var.use_aws ? {} : module.data_share_processors
  service_account_id = google_service_account.sum_part_bucket_writer.name
  role               = "roles/iam.serviceAccountTokenCreator"
  member             = "serviceAccount:${each.value.gcp_service_account_email}"
}

# GCP services we must enable to use Workload Identity Pool
resource "google_project_service" "sts" {
  count   = var.use_aws ? 1 : 0
  service = "sts.googleapis.com"
}

resource "google_project_service" "iamcredentials" {
  count              = var.use_aws ? 1 : 0
  service            = "iamcredentials.googleapis.com"
  disable_on_destroy = false
}

# If running EKS, we must create a Workload Identity Pool and Workload Identity
# Pool Provider to federate accounts.google.com with sts.amazonaws.com so that
# our data share processor AWS IAM roles will be able to impersonate the sum
# part bucket writer GCP service account.
resource "google_iam_workload_identity_pool" "aws_identity_federation" {
  count                     = var.use_aws ? 1 : 0
  provider                  = google-beta
  workload_identity_pool_id = "aws-identity-federation"
}

# A Workload Identity Pool *Provider* goes with the Workload Identity Pool and
# is bound to our AWS account ID by its numeric identifier. We don't specify any
# additional conditions or policies here, and instead use a GCP SA IAM binding
# on the sum part bucket writer.
resource "google_iam_workload_identity_pool_provider" "aws_identity_federation" {
  count                              = var.use_aws ? 1 : 0
  provider                           = google-beta
  workload_identity_pool_provider_id = "aws-identity-federation"
  workload_identity_pool_id          = google_iam_workload_identity_pool.aws_identity_federation[0].workload_identity_pool_id
  aws {
    account_id = data.aws_caller_identity.current.account_id
  }
}

resource "google_service_account_iam_binding" "data_share_processors_to_sum_part_bucket_writer_workload_identity_user" {
  count              = var.use_aws ? 1 : 0
  service_account_id = google_service_account.sum_part_bucket_writer.name
  role               = "roles/iam.workloadIdentityUser"
  # This principal allows an IAM role to impersonate the service account via the
  # Workload Identity Pool
  # https://cloud.google.com/iam/docs/access-resources-aws#impersonate
  members = [
    for v in module.data_share_processors : join("/", [
      "principalSet://iam.googleapis.com/projects",
      data.google_project.current.number,
      "locations/global/workloadIdentityPools",
      google_iam_workload_identity_pool.aws_identity_federation[0].workload_identity_pool_id,
      "attribute.aws_role/arn:aws:sts::${data.aws_caller_identity.current.account_id}:assumed-role",
      v.aws_iam_role.name,
    ])
  ]
}

locals {
  gcp_workload_identity_pool_provider = var.use_aws ? (
    join("/", [
      "//iam.googleapis.com/projects",
      data.google_project.current.number,
      "locations/global/workloadIdentityPools",
      google_iam_workload_identity_pool.aws_identity_federation[0].workload_identity_pool_id,
      "providers",
      google_iam_workload_identity_pool_provider.aws_identity_federation[0].workload_identity_pool_provider_id,
    ])
    ) : (
    ""
  )
}

module "fake_server_resources" {
  count                                = local.is_env_with_ingestor ? 1 : 0
  source                               = "./modules/fake_server_resources"
  gcp_region                           = var.gcp_region
  gcp_project                          = var.gcp_project
  environment                          = var.environment
  ingestor_pairs                       = local.locality_ingestor_pairs
  facilitator_manifest_base_url        = local.manifest.base_url
  container_registry                   = var.container_registry
  facilitator_image                    = var.facilitator_image
  facilitator_version                  = var.facilitator_version
  manifest_bucket                      = local.manifest.bucket
  other_environment                    = var.test_peer_environment.env_without_ingestor
  sum_part_bucket_writer_name          = google_service_account.sum_part_bucket_writer.name
  sum_part_bucket_writer_email         = google_service_account.sum_part_bucket_writer.email
  aggregate_queues                     = { for v in module.data_share_processors : v.data_share_processor_name => v.aggregate_queue }
  role_permissions_boundary_policy_arn = var.role_permissions_boundary_policy_arn
}

module "portal_server_resources" {
  count                        = local.deployment_has_ingestor ? 1 : 0
  source                       = "./modules/portal_server_resources"
  manifest_bucket              = local.manifest.bucket
  use_aws                      = var.use_aws
  gcp_region                   = var.gcp_region
  environment                  = var.environment
  other_environment            = local.is_env_with_ingestor ? var.test_peer_environment.env_without_ingestor : var.test_peer_environment.env_with_ingestor
  sum_part_bucket_writer_email = google_service_account.sum_part_bucket_writer.email
}

module "custom_metrics" {
  source            = "./modules/custom_metrics"
  environment       = var.environment
  use_aws           = var.use_aws
  eks_oidc_provider = var.use_aws ? module.eks[0].oidc_provider : { url = "", arn = "" }
}

# The monitoring module is disabled for now because it needs some AWS tweaks
# (wire up an EBS volume for metrics storage and forward SQS metrics into
# Prometheus). I'm commenting it out instead of putting in a count variable
# because this way we don't have to `terraform state mv` its resources into
# place when we turn it on.
module "monitoring" {
  source      = "./modules/monitoring"
  environment = var.environment
  use_aws     = var.use_aws
  gcp = {
    region  = var.gcp_region
    project = var.gcp_project
  }
  victorops_routing_key = var.victorops_routing_key
  aggregation_period    = var.default_aggregation_period
  eks_oidc_provider     = var.use_aws ? module.eks[0].oidc_provider : { url = "", arn = "" }

  prometheus_server_persistent_disk_size_gb = var.prometheus_server_persistent_disk_size_gb

  prometheus_helm_chart_version           = var.prometheus_helm_chart_version
  grafana_helm_chart_version              = var.grafana_helm_chart_version
  cloudwatch_exporter_helm_chart_version  = var.cloudwatch_exporter_helm_chart_version
  stackdriver_exporter_helm_chart_version = var.stackdriver_exporter_helm_chart_version
}

resource "kubernetes_namespace_v1" "nfs" {
  count = var.enable_heap_profiles ? 1 : 0
  metadata {
    name = "nfs"
  }
}

module "nfs_server" {
  count     = var.enable_heap_profiles ? 1 : 0
  source    = "./modules/nfs_server"
  namespace = kubernetes_namespace_v1.nfs[0].metadata[0].name
}

data "external" "git-describe" {
  program = ["bash", "-c", "echo {\\\"result\\\":\\\"$(git describe --always --dirty)\\\"}"]
}

output "git-describe" {
  value = data.external.git-describe.result.result
}
