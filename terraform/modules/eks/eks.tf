variable "environment" {
  type = string
}

variable "aws_region" {
  type = string
}

# This cluster role binding references the built-in cluster role "view" and
# defines a group that can then be referenced in the kube-system/aws-auth config
# map.
resource "kubernetes_cluster_role_binding" "read_only" {
  metadata {
    name = "read-only"
  }

  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = "view"
  }

  subject {
    api_group = "rbac.authorization.k8s.io"
    kind      = "Group"
    name      = "read-only"
  }
}

data "aws_eks_cluster" "cluster" {
  name = "prio-${var.environment}"
}

data "aws_eks_cluster_auth" "cluster_auth" {
  name = "prio-${var.environment}"
}

# Provider aws lacks data.aws_iam_openid_connect_provider, so we must construct
# the OIDC provider ARN
# https://github.com/hashicorp/terraform-provider-aws/issues/17747
data "aws_partition" "current" {}
data "aws_caller_identity" "current" {}

locals {
  oidc_provider_url = replace(data.aws_eks_cluster.cluster.identity[0].oidc[0].issuer, "https://", "")
  oidc_provider_arn = "arn:${data.aws_partition.current.id}:iam::${data.aws_caller_identity.current.account_id}:oidc-provider/${local.oidc_provider_url}"
}

output "oidc_provider" {
  value = {
    url = local.oidc_provider_url
    arn = local.oidc_provider_arn
  }
}

output "cluster_name" {
  value = data.aws_eks_cluster.cluster.name
}

output "cluster_endpoint" {
  value = data.aws_eks_cluster.cluster.endpoint
}

output "certificate_authority_data" {
  value = data.aws_eks_cluster.cluster.certificate_authority[0].data
}

output "token" {
  value = data.aws_eks_cluster_auth.cluster_auth.token
}
