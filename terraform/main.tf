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

variable "peer_share_processor_names" {
  type = list(string)
}

locals {
  resource_prefix = "prio-${var.environment}"
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
  # aws_s3_bucket resources will be created in the region specified here
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

module "gke" {
  source          = "./modules/gke"
  environment     = var.environment
  resource_prefix = local.resource_prefix
  gcp_region      = var.gcp_region
  gcp_project     = var.gcp_project
  machine_type    = var.machine_type
}

module "facilitator" {
  for_each                  = toset(var.peer_share_processor_names)
  source                    = "./modules/facilitator"
  environment               = var.environment
  peer_share_processor_name = each.key
  gcp_project               = var.gcp_project

  depends_on = [module.gke]
}
output "gke_kubeconfig" {
  value = "Run this command to update your kubectl config: gcloud container clusters get-credentials ${module.gke.cluster_name} --region ${var.gcp_region}"
}
