variable "environment" {
  type        = string
  description = <<DESCRIPTION
The name of the prio-server environment this account will host.
DESCRIPTION
}

variable "gcp_region" {
  type        = string
  description = <<DESCRIPTION
The GCP region in which the Terraform state will be stored.
DESCRIPTION
}

variable "gcp_project" {
  type        = string
  description = <<DESCRIPTION
Name of the GCP project to be created and in which resources will be created.
DESCRIPTION
}

variable "aws_region" {
  type        = string
  description = <<DESCRIPTION
The AWS region in which resources should be created.
DESCRIPTION
}

variable "use_aws" {
  type        = bool
  description = <<DESCRIPTION
If true, the environment is deployed to AWS/EKS. If false, the environment is
deployed to GCP/GKE.
DESCRIPTION
}

variable "cluster_settings" {
  type = object({
    initial_node_count = number
    min_node_count     = number
    max_node_count     = number
    gcp_machine_type   = string
    aws_machine_types  = list(string)
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

Note that on AWS, there will always be at least three worker nodes in an "on
demand" node pool, and the `{min,max}_node_count` values only govern the size of
an additional "spot" node pool.
DESCRIPTION
}

terraform {
  backend "gcs" {}

  required_version = ">= 1.1.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 3.70.0"
    }
    google = {
      source = "hashicorp/google"
      # Keep this version in sync with provider google-beta
      version = "~> 4.5.0"
    }
    google-beta = {
      source = "hashicorp/google-beta"
      # Keep this version in sync with provider google
      version = "~> 4.10.0"
    }
  }
}

# AWS provider is configured via environment variables that get set by aws-mfa
# script
provider "aws" {
  region = var.aws_region

  default_tags {
    tags = {
      "prio-env" = var.environment
    }
  }
}

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

module "gke" {
  source           = "./modules/gke"
  count            = var.use_aws ? 0 : 1
  environment      = var.environment
  resource_prefix  = "prio-${var.environment}"
  gcp_region       = var.gcp_region
  gcp_project      = var.gcp_project
  cluster_settings = var.cluster_settings
}

module "eks" {
  source           = "./modules/eks"
  count            = var.use_aws ? 1 : 0
  environment      = var.environment
  resource_prefix  = "prio-${var.environment}"
  cluster_settings = var.cluster_settings
}
