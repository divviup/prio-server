variable "infra_name" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "machine_type" {
  type    = string
  default = "e2.small"
}

variable "peer_share_processor_names" {
  type = list(string)
}

variable "container_registry" {
  type = string
}

variable "execution_manager_image" {
  type = string
}

variable "execution_manager_version" {
  type = string
}

locals {
  resource_prefix = "prio-${var.infra_name}"
}

terraform {
  backend "gcs" {}

  required_version = ">= 0.13.3"
}

data "terraform_remote_state" "state" {
  backend = "gcs"

  workspace = "${var.infra_name}-${var.gcp_region}"

  config = {
    bucket = "${var.infra_name}-${var.gcp_region}-prio-terraform"
  }
}

data "google_client_config" "current" {}

provider "google" {
  # by default this will use "Application Default Credentials". Run `gcloud auth
  # application-default login` to generate them.
  # https://www.terraform.io/docs/providers/google/guides/provider_reference.html#credentials
  region  = var.gcp_region
  project = var.gcp_project
}

provider "kubernetes" {
  host                   = module.gke.cluster_endpoint
  cluster_ca_certificate = base64decode(module.gke.certificate_authority_data)
  token                  = data.google_client_config.current.access_token
  load_config_file       = false
}

module "gke" {
  source          = "./modules/gke"
  infra_name      = var.infra_name
  resource_prefix = local.resource_prefix
  gcp_region      = var.gcp_region
  machine_type    = var.machine_type
}

module "kubernetes" {
  source                     = "./modules/kubernetes/"
  container_registry         = "${var.container_registry}"
  execution_manager_image    = var.execution_manager_image
  execution_manager_version  = var.execution_manager_version
  peer_share_processor_names = var.peer_share_processor_names
  infra_name                 = var.infra_name

  depends_on = [module.gke]
}

output "gke_kubeconfig" {
  value = "Run this command to update your kubectl config: gcloud container clusters get-credentials ${module.gke.cluster_name} --region ${var.gcp_region}"
}
