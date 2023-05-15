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

variable "allowed_aws_account_ids" {
  type        = list(string)
  default     = null
  description = <<DESCRIPTION
A list of AWS account IDs that are allowed to be used. Set this in an
environment's variables to prevent accidentally using the environment with
the wrong AWS configuration profile. This variable may be omitted, in which
case no AWS account ID check is performed. If a list of account IDs is
given, the provider will check the account of the active credentials, and
return an error if it is not listed here.
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
  - On GKE, the above node counts are interpreted as the number of nodes per
    zone. On EKS, the node counts are interpreted as totals across the
    region.
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

terraform {
  backend "gcs" {}

  required_version = ">= 1.3.1"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "4.66.1"
    }
    google = {
      source = "hashicorp/google"
      # Keep this version in sync with provider google-beta
      version = "4.63.1"
    }
    google-beta = {
      source = "hashicorp/google-beta"
      # Keep this version in sync with provider google
      version = "4.64.0"
    }
    external = {
      source  = "hashicorp/external"
      version = "2.3.1"
    }
    random = {
      source  = "hashicorp/random"
      version = "3.5.1"
    }
    tls = {
      source  = "hashicorp/tls"
      version = "4.0.4"
    }
  }
}

# AWS provider is configured via environment variables that get set by aws-mfa
# script
provider "aws" {
  region              = var.aws_region
  allowed_account_ids = var.allowed_aws_account_ids

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

output "google_kms_key_ring_id" {
  value = var.use_aws ? "" : module.gke[0].google_kms_key_ring_id
}

data "external" "git-describe" {
  program = ["bash", "-c", "echo {\\\"result\\\":\\\"$(git describe --always --dirty)\\\"}"]
}

output "git-describe" {
  value = data.external.git-describe.result.result
}
