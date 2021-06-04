variable "environment" {
  type = string
}

variable "resource_prefix" {
  type = string
}

variable "cluster_settings" {
  type = object({
    initial_node_count = number
    min_node_count     = number
    max_node_count     = number
    gcp_machine_type   = string
    aws_machine_types  = list(string)
  })
}

data "aws_region" "current" {}

# Private virtual network that worker nodes will be created in.
resource "aws_vpc" "cluster" {
  cidr_block = "10.0.0.0/16"
  # DNS support is required for EKS. enable_dns_hostnames only matters for
  # instances that get public IPs, which we should not have, but there's no harm
  # in turning it on.
  # https://docs.aws.amazon.com/eks/latest/userguide/network_reqs.html
  enable_dns_support   = "true"
  enable_dns_hostnames = "true"

  tags = {
    Name = var.resource_prefix
  }
}

# Allow egress to the Internet from the VPC, though private subnets still need
# NAT gateways
resource "aws_internet_gateway" "cluster_internet" {
  vpc_id = aws_vpc.cluster.id

  tags = {
    Name = var.resource_prefix
  }
}

# Though we create our worker nodes in private subnets, we need one public
# subnet (i.e., one with a default route through an Internet gateway) in which
# we can create NAT gateways.
resource "aws_subnet" "public" {
  vpc_id = aws_vpc.cluster.id
  # We use the first /24 from the VPC for the public subnet
  cidr_block = cidrsubnet(aws_vpc.cluster.cidr_block, 8, 0)

  tags = {
    Name = var.resource_prefix
  }
}

# Route all traffic through the Internet gateway so that the NAT gateways we
# create in the public subnet can reach the Internet.
resource "aws_route_table" "internet_egress" {
  vpc_id = aws_vpc.cluster.id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.cluster_internet.id
  }

  tags = {
    Name = "${var.resource_prefix}-internet-egress"
  }
}

resource "aws_route_table_association" "internet_egress" {
  subnet_id      = aws_subnet.public.id
  route_table_id = aws_route_table.internet_egress.id
}

# Discover available availability zones and create subnets in the first three
data "aws_availability_zones" "azs" {
  state = "available"
}

locals {
  # We want workers across up to three AZs if available, but some AWS regions
  # only have two AZs.
  az_count          = min(3, length(data.aws_availability_zones.azs.names))
  oidc_provider_url = replace(aws_iam_openid_connect_provider.oidc.url, "https://", "")
}

module "subnets" {
  source = "./subnet/"

  count                  = local.az_count
  resource_prefix        = var.resource_prefix
  availability_zone_name = data.aws_availability_zones.azs.names[count.index]
  availability_zone_id   = data.aws_availability_zones.azs.zone_ids[count.index]
  vpc_id                 = aws_vpc.cluster.id
  public_subnet          = aws_subnet.public.id
  # Allocate a /24 from the VPC's /16 for this subnet. Start at count.index + 1
  # because we used the first /24 for the public subnet.
  subnet_cidr = cidrsubnet(aws_vpc.cluster.cidr_block, 8, count.index + 1)

  depends_on = [aws_internet_gateway.cluster_internet]
}

# To use AWS services without the public internet, we place VPC endpoints in the
# cluster VPC's private subnets (or in the private subnet route table in the
# case of S3).
resource "aws_vpc_endpoint" "ecr_dkr" {
  vpc_id              = aws_vpc.cluster.id
  service_name        = "com.amazonaws.${data.aws_region.current.name}.ecr.dkr"
  vpc_endpoint_type   = "Interface"
  private_dns_enabled = true
  subnet_ids          = module.subnets[*].id
  security_group_ids  = [aws_eks_cluster.cluster.vpc_config[0].cluster_security_group_id]
}

resource "aws_vpc_endpoint" "ecr_api" {
  vpc_id              = aws_vpc.cluster.id
  service_name        = "com.amazonaws.${data.aws_region.current.name}.ecr.api"
  vpc_endpoint_type   = "Interface"
  private_dns_enabled = true
  subnet_ids          = module.subnets[*].id
  security_group_ids  = [aws_eks_cluster.cluster.vpc_config[0].cluster_security_group_id]
}

resource "aws_vpc_endpoint" "ec2" {
  vpc_id              = aws_vpc.cluster.id
  service_name        = "com.amazonaws.${data.aws_region.current.name}.ec2"
  vpc_endpoint_type   = "Interface"
  private_dns_enabled = true
  subnet_ids          = module.subnets[*].id
  security_group_ids  = [aws_eks_cluster.cluster.vpc_config[0].cluster_security_group_id]
}

resource "aws_vpc_endpoint" "s3" {
  vpc_id            = aws_vpc.cluster.id
  service_name      = "com.amazonaws.${data.aws_region.current.name}.s3"
  vpc_endpoint_type = "Gateway"
  route_table_ids   = module.subnets[*].route_table_id
}

resource "aws_vpc_endpoint" "sts" {
  vpc_id              = aws_vpc.cluster.id
  service_name        = "com.amazonaws.${data.aws_region.current.name}.sts"
  vpc_endpoint_type   = "Interface"
  private_dns_enabled = true
  subnet_ids          = module.subnets[*].id
  security_group_ids  = [aws_eks_cluster.cluster.vpc_config[0].cluster_security_group_id]
}

resource "aws_vpc_endpoint" "autoscaling" {
  vpc_id              = aws_vpc.cluster.id
  service_name        = "com.amazonaws.${data.aws_region.current.name}.autoscaling"
  vpc_endpoint_type   = "Interface"
  private_dns_enabled = true
  subnet_ids          = module.subnets[*].id
  security_group_ids  = [aws_eks_cluster.cluster.vpc_config[0].cluster_security_group_id]
}

resource "aws_vpc_endpoint" "cloudwatch_logs" {
  vpc_id              = aws_vpc.cluster.id
  service_name        = "com.amazonaws.${data.aws_region.current.name}.logs"
  vpc_endpoint_type   = "Interface"
  private_dns_enabled = true
  subnet_ids          = module.subnets[*].id
  security_group_ids  = [aws_eks_cluster.cluster.vpc_config[0].cluster_security_group_id]
}

# This role lets the managed Kubernetes cluster manage AWS resources. It is
# identical to the roles created for any other EKS cluster.
resource "aws_iam_role" "cluster_role" {
  name = "${var.resource_prefix}-cluster"
  managed_policy_arns = [
    "arn:aws:iam::aws:policy/AmazonEKSClusterPolicy",
  ]

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action = ["sts:AssumeRole"]
        Effect = "Allow"
        Principal = {
          Service = "eks.amazonaws.com"
        }
      }
    ]
  })
}

# KMS symmetric key, used to encrypt Kubernetes secrets at rest
resource "aws_kms_key" "etcd_secrets" {
  description = "Encryption at rest for secrets in ${var.resource_prefix} cluster"
  key_usage   = "ENCRYPT_DECRYPT"

  tags = {
    Name = "${var.resource_prefix}-secrets-encryption"
  }
}

resource "aws_eks_cluster" "cluster" {
  name     = var.resource_prefix
  role_arn = aws_iam_role.cluster_role.arn
  version  = "1.20"
  # Send cluster logs to CloudWatch Logs
  enabled_cluster_log_types = ["api", "audit", "authenticator", "controllerManager", "scheduler"]

  vpc_config {
    # Enable public access so operators can ereach the API server with kubectl
    # or terraform, and private access so that worker nodes can reach the API
    # server over the cluster VPC. EKS automatically creates a security group
    # allowing traffic between worker nodes and the API server.
    endpoint_public_access  = true
    endpoint_private_access = true
    # Subnets over which worker nodes reach the API server
    subnet_ids = module.subnets[*].id
  }

  # Configure at-rest encryption of Kubernetes secrets
  encryption_config {
    # EKS only lets us encrypt secrets -- why not all of etcd?
    resources = ["secrets"]
    provider {
      key_arn = aws_kms_key.etcd_secrets.arn
    }
  }
}

data "tls_certificate" "cluster" {
  url = aws_eks_cluster.cluster.identity[0].oidc[0].issuer
}

# EKS allows mapping Kubernetes service accounts to AWS IAM roles. We configure
# the cluster to trust an OIDC provider for AWS STS.
# https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html
resource "aws_iam_openid_connect_provider" "oidc" {
  url             = aws_eks_cluster.cluster.identity[0].oidc[0].issuer
  client_id_list  = ["sts.amazonaws.com"]
  thumbprint_list = [data.tls_certificate.cluster.certificates[0].sha1_fingerprint]
}

# This role lets the managed Kubernetes worker nodes access ECR and EC2
# resources.
resource "aws_iam_role" "worker_node_role" {
  name = "${var.resource_prefix}-worker-node"
  managed_policy_arns = [
    "arn:aws:iam::aws:policy/AmazonEKSWorkerNodePolicy",
    "arn:aws:iam::aws:policy/AmazonEC2ContainerRegistryReadOnly",
    "arn:aws:iam::aws:policy/AmazonEKS_CNI_Policy",
  ]

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action = ["sts:AssumeRole"]
        Effect = "Allow"
        Principal = {
          Service = "ec2.amazonaws.com"
        }
      }
    ]
  })
}

resource "aws_eks_node_group" "node_group" {
  node_group_name = var.resource_prefix
  cluster_name    = aws_eks_cluster.cluster.name
  node_role_arn   = aws_iam_role.worker_node_role.arn
  # Worker nodes will be distributed across the AZs that the subnets are in
  subnet_ids     = module.subnets[*].id
  capacity_type  = "SPOT"
  instance_types = var.cluster_settings.aws_machine_types

  scaling_config {
    desired_size = var.cluster_settings.initial_node_count
    max_size     = var.cluster_settings.max_node_count
    min_size     = var.cluster_settings.min_node_count
  }

  lifecycle {
    ignore_changes = [
      # This will change as the node group autoscales. Ignore it so that
      # Terraform doesn't resize the node group on apply.
      scaling_config[0].desired_size
    ]
  }
}

# The AWS VPC CNI plugin allows pods to have the same IP in the pod as they do
# on the VPC network.
# https://docs.aws.amazon.com/eks/latest/userguide/managing-vpc-cni.html
# AWS docs say we should configure it to assume an IAM role that just has
# AmazonEKS_CNI_Policy, but empirically that doesn't actually work so instead we
# allow it to implicitly inherit the worker-node role. It seems that this might
# be because of issues with how the VPC CNI plugin is reaching STS to perform
# sts:AssumeRole calls.
# https://github.com/aws/amazon-vpc-cni-k8s/issues/1473
# https://github.com/aws/amazon-vpc-cni-k8s/issues/647
resource "aws_eks_addon" "vpc_cni" {
  cluster_name = aws_eks_cluster.cluster.name
  addon_name   = "vpc-cni"
}

data "aws_eks_cluster_auth" "cluster" {
  name = aws_eks_cluster.cluster.name
}

output "cluster_name" {
  value = aws_eks_cluster.cluster.name
}

output "cluster_endpoint" {
  value = aws_eks_cluster.cluster.endpoint
}

output "certificate_authority_data" {
  value = aws_eks_cluster.cluster.certificate_authority[0].data
}

output "token" {
  value = data.aws_eks_cluster_auth.cluster.token
}

output "oidc_provider" {
  value = {
    url = local.oidc_provider_url
    arn = aws_iam_openid_connect_provider.oidc.arn
  }
}
