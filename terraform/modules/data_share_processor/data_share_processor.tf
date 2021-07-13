variable "ingestor" {
  type = string
}

variable "data_share_processor_name" {
  type = string
}

variable "environment" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "use_aws" {
  type = bool
}

variable "pure_gcp" {
  type = bool
}

variable "aws_region" {
  type = string
}

variable "kubernetes_namespace" {
  type = string
}

variable "packet_decryption_key_kubernetes_secret" {
  type = string
}

variable "certificate_domain" {
  type = string
}

variable "ingestor_manifest_base_url" {
  type = string
}

variable "peer_share_processor_manifest_base_url" {
  type = string
}

variable "own_manifest_base_url" {
  type = string
}

variable "remote_bucket_writer_gcp_service_account_email" {
  type = string
}

variable "portal_server_manifest_base_url" {
  type = string
}

variable "is_first" {
  type = bool
}

variable "intake_max_age" {
  type = string
}

variable "aggregation_period" {
  type = string
}

variable "aggregation_grace_period" {
  type = string
}

variable "kms_keyring" {
  type = string
}

variable "pushgateway" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "workflow_manager_image" {
  type = string
}

variable "workflow_manager_version" {
  type = string
}

variable "facilitator_image" {
  type = string
}

variable "facilitator_version" {
  type = string
}

variable "intake_worker_count" {
  type = number
}

variable "aggregate_worker_count" {
  type = number
}

variable "eks_oidc_provider" {
  type = object({
    arn = string
    url = string
  })
  default = {
    arn = ""
    url = ""
  }
}

variable "gcp_workload_identity_pool_provider" {
  type = string
}

# We need the ingestion server's manifest so that we can discover the GCP
# service account it will use to upload ingestion batches. Some ingestors
# (Apple) are singletons, and advertise a single global manifest which contains
# the single GCP SA used by that server to send ingestion batches. Others
# (Google) operate a distinct instance per locality, using a distinct GCP SA for
# each. We first check for a global manifest, and fall back to a specific
# manifest if it cannot be found. A Terraform data.http block would cause the
# entire plan or apply to fail if the specified URL can't be loaded, so we use
# this data.external block to check the HTTP status.
data "external" "global_manifest_http_status" {
  program = [
    "curl",
    "--silent",
    "--output", "/dev/null",
    # Terraform insists that the output of the program be JSON so that it can be
    # parsed into a Terraform map.
    "--write-out", "{ \"http_code\": \"%%{http_code}\" }",
    "https://${var.ingestor_manifest_base_url}/global-manifest.json"
  ]
}

data "http" "ingestor_global_manifest" {
  count = local.ingestor_global_manifest_exists ? 1 : 0
  url   = "https://${var.ingestor_manifest_base_url}/global-manifest.json"
}

data "http" "ingestor_specific_manifest" {
  count = local.ingestor_global_manifest_exists ? 0 : 1
  url   = "https://${var.ingestor_manifest_base_url}/${var.data_share_processor_name}-manifest.json"
}

data "http" "peer_share_processor_global_manifest" {
  url = "https://${var.peer_share_processor_manifest_base_url}/global-manifest.json"
}

locals {
  ingestor_global_manifest_exists = data.external.global_manifest_http_status.result.http_code == "200"
  ingestor_server_identity = local.ingestor_global_manifest_exists ? (
    jsondecode(data.http.ingestor_global_manifest[0].body).server-identity
    ) : (
    jsondecode(data.http.ingestor_specific_manifest[0].body).server-identity
  )
  ingestor_gcp_service_account_id = lookup(local.ingestor_server_identity, "gcp-service-account-id", "")
  ingestor_aws_role_arn           = lookup(local.ingestor_server_identity, "aws-iam-entity", "")

  resource_prefix = "prio-${var.environment}-${var.data_share_processor_name}"

  peer_share_processor_server_identity               = jsondecode(data.http.peer_share_processor_global_manifest.body).server-identity
  peer_share_processor_gcp_service_account_email     = local.peer_share_processor_server_identity.gcp-service-account-email
  peer_share_processor_gcp_service_account_unique_id = local.peer_share_processor_server_identity.gcp-service-account-id

  # If running EKS, we impersonate a GCP SA shared across the whole env to write
  # to the peer's validation buckets in S3.
  # If running in impure GCP, we assume the bucket writer IAM role we created.
  # If running in pure GCP, we will discover a bucket writer IAM role created by
  # the peer at runtime, in their specific manifest, but must impersonate a GCP
  # SA in order to obtain identity tokens.
  remote_validation_bucket_writer = var.use_aws ? {
    identity                                      = var.remote_bucket_writer_gcp_service_account_email
    gcp_sa_to_impersonate_while_assuming_identity = ""
    } : var.pure_gcp ? {
    identity                                      = ""
    gcp_sa_to_impersonate_while_assuming_identity = var.remote_bucket_writer_gcp_service_account_email
    } : {
    identity                                      = aws_iam_role.bucket_role[0].arn
    gcp_sa_to_impersonate_while_assuming_identity = ""
  }

  ingestion_bucket_name = "${local.resource_prefix}-ingestion"
  ingestion_bucket_url = var.use_aws ? (
    "s3://${var.aws_region}/${local.ingestion_bucket_name}"
    ) : (
    "gs://${local.ingestion_bucket_name}"
  )
  peer_validation_bucket_name = "${local.resource_prefix}-peer-validation"
  peer_validation_bucket_url = var.use_aws ? (
    "s3://${var.aws_region}/${local.peer_validation_bucket_name}"
    ) : (
    "gs://${local.peer_validation_bucket_name}"
  )
  own_validation_bucket_name = "${local.resource_prefix}-own-validation"
  own_validation_bucket_url = var.use_aws ? (
    "s3://${var.aws_region}/${local.own_validation_bucket_name}"
    ) : (
    "gs://${local.own_validation_bucket_name}"
  )

  intake_queue    = var.use_aws ? module.sns_sqs["intake"].queue : module.pubsub["intake"].queue
  aggregate_queue = var.use_aws ? module.sns_sqs["aggregate"].queue : module.pubsub["aggregate"].queue
}

# For data share processors that run in GKE but are not pure GKE, or ones that
# run in EKS, we create an IAM role to enable writing to the peer's validation
# buckets in S3.
# https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_oidc.html
resource "aws_iam_role" "bucket_role" {
  count = var.use_aws || !var.pure_gcp ? 1 : 0

  name = "${local.resource_prefix}-bucket-role"
  # Since azp is set in the auth token GCP generates, we must check oaud in the
  # role assumption policy, and the value must match what we request when
  # requesting identity tokens from the GCP API
  # https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_iam-condition-keys.html
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          Federated = "accounts.google.com"
        }
        Action = "sts:AssumeRoleWithWebIdentity"
        Condition = {
          StringEquals = {
            # When running in EKS, we allow the peer's GCP service account to
            # assume this role to write to our buckets.
            # When running in non-pure GCP, we allow our data share processor's
            # GCP service account to assume the role to write to the peer's
            # validation buckets.
            # When running in pure GCP, we don't create this role at all.
            "accounts.google.com:sub"  = var.use_aws ? local.peer_share_processor_gcp_service_account_unique_id : module.kubernetes.gcp_service_account_unique_id
            "accounts.google.com:oaud" = "sts.amazonaws.com/gke-identity-federation"
          }
        }
      }
    ]
  })
}

# The bucket_role defined above is created in our AWS account and used to
# access S3 buckets in a peer's AWS account. Besides _their_ bucket having a
# policy that grants us access, we need to make sure _our_ role allows use of S3
# API, hence this policy. Note it is not the same as the role assumption policy
# defined inline in aws_iam_role.bucket_role.
# TODO: Make this policy as restrictive as possible:
# https://github.com/abetterinternet/prio-server/issues/140
resource "aws_iam_role_policy" "bucket_role_policy" {
  count = var.use_aws && !var.pure_gcp ? 1 : 0

  name = "${local.resource_prefix}-bucket-role-policy"
  role = aws_iam_role.bucket_role[0].id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action   = ["s3:*"]
        Effect   = "Allow"
        Resource = "*"
      }
    ]
  })
}

# If our ingestion bucket is in S3 and this data share processor's ingestion
# server does not have an AWS IAM identity, then we must create an AWS IAM role
# for it to assume using its GCP service account.
resource "aws_iam_role" "ingestion_bucket_writer" {
  count = (var.use_aws && local.ingestor_aws_role_arn == "") ? 1 : 0

  name = "${local.resource_prefix}-ingestion-bucket-writer"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          Federated = "accounts.google.com"
        }
        Action = "sts:AssumeRoleWithWebIdentity"
        Condition = {
          StringEquals = {
            "accounts.google.com:sub" = local.ingestor_gcp_service_account_id
          }
        }
      }
    ]
  })
}

module "cloud_storage_aws" {
  count  = var.use_aws ? 1 : 0
  source = "../../modules/cloud_storage_aws"

  resource_prefix       = local.resource_prefix
  bucket_reader         = module.kubernetes.aws_iam_role.arn
  ingestion_bucket_name = local.ingestion_bucket_name
  # If the ingestor has a GCP service account, grant write access to
  # aws_iam_role.ingestion_bucket_writer, the IAM role configured to allow
  # assumption by the GCP SA. If the ingestor has an AWS IAM role, just grant
  # permission to it.
  ingestion_bucket_writer = local.ingestor_aws_role_arn == "" ? (
    aws_iam_role.ingestion_bucket_writer[0].arn
    ) : (
    local.ingestor_aws_role_arn
  )
  peer_validation_bucket_name   = local.peer_validation_bucket_name
  peer_validation_bucket_writer = aws_iam_role.bucket_role[0].arn
  own_validation_bucket_name    = "${local.resource_prefix}-own-validation"
  own_validation_bucket_writer  = module.kubernetes.aws_iam_role.arn
}

module "cloud_storage_gcp" {
  count  = var.use_aws ? 0 : 1
  source = "../../modules/cloud_storage_gcp"

  resource_prefix               = local.resource_prefix
  gcp_region                    = var.gcp_region
  kms_keyring                   = var.kms_keyring
  bucket_reader                 = module.kubernetes.gcp_service_account_email
  ingestion_bucket_name         = local.ingestion_bucket_name
  ingestion_bucket_writer       = local.ingestor_server_identity.gcp-service-account-email
  peer_validation_bucket_name   = local.peer_validation_bucket_name
  peer_validation_bucket_writer = local.peer_share_processor_gcp_service_account_email
  own_validation_bucket_name    = "${local.resource_prefix}-own-validation"
  own_validation_bucket_writer  = module.kubernetes.gcp_service_account_email
}

module "pubsub" {
  for_each                   = toset(var.use_aws ? [] : ["intake", "aggregate"])
  source                     = "../../modules/pubsub"
  environment                = var.environment
  data_share_processor_name  = var.data_share_processor_name
  publisher_service_account  = module.kubernetes.gcp_service_account_email
  subscriber_service_account = module.kubernetes.gcp_service_account_email
  task                       = each.key
}

module "sns_sqs" {
  for_each                  = toset(var.use_aws ? ["intake", "aggregate"] : [])
  source                    = "../../modules/sns_sqs"
  environment               = var.environment
  data_share_processor_name = var.data_share_processor_name
  publisher_iam_role        = module.kubernetes.aws_iam_role.arn
  subscriber_iam_role       = module.kubernetes.aws_iam_role.arn
  task                      = each.key
}

module "kubernetes" {
  source                                  = "../../modules/kubernetes/"
  data_share_processor_name               = var.data_share_processor_name
  ingestor                                = var.ingestor
  environment                             = var.environment
  kubernetes_namespace                    = var.kubernetes_namespace
  ingestion_bucket                        = local.ingestion_bucket_url
  ingestor_manifest_base_url              = var.ingestor_manifest_base_url
  packet_decryption_key_kubernetes_secret = var.packet_decryption_key_kubernetes_secret
  peer_manifest_base_url                  = var.peer_share_processor_manifest_base_url
  remote_peer_validation_bucket_identity  = local.remote_validation_bucket_writer
  peer_validation_bucket                  = local.peer_validation_bucket_url
  own_validation_bucket                   = local.own_validation_bucket_url
  own_manifest_base_url                   = var.own_manifest_base_url
  sum_part_bucket_service_account_email   = var.remote_bucket_writer_gcp_service_account_email
  portal_server_manifest_base_url         = var.portal_server_manifest_base_url
  is_first                                = var.is_first
  intake_max_age                          = var.intake_max_age
  aggregation_period                      = var.aggregation_period
  aggregation_grace_period                = var.aggregation_grace_period
  pushgateway                             = var.pushgateway
  container_registry                      = var.container_registry
  workflow_manager_image                  = var.workflow_manager_image
  workflow_manager_version                = var.workflow_manager_version
  facilitator_image                       = var.facilitator_image
  facilitator_version                     = var.facilitator_version
  intake_queue                            = local.intake_queue
  aggregate_queue                         = local.aggregate_queue
  intake_worker_count                     = var.intake_worker_count
  aggregate_worker_count                  = var.aggregate_worker_count
  use_aws                                 = var.use_aws
  eks_oidc_provider                       = var.eks_oidc_provider
  aws_region                              = var.aws_region
  gcp_workload_identity_pool_provider     = var.gcp_workload_identity_pool_provider
}

output "data_share_processor_name" {
  value = var.data_share_processor_name
}

output "ingestor_name" {
  value = var.ingestor
}

output "kubernetes_namespace" {
  value = var.kubernetes_namespace
}

output "certificate_fqdn" {
  value = "${var.kubernetes_namespace}.${var.certificate_domain}"
}

output "gcp_service_account_email" {
  value = module.kubernetes.gcp_service_account_email
}

output "aws_iam_role" {
  value = module.kubernetes.aws_iam_role
}

output "specific_manifest" {
  value = var.pure_gcp ? {
    format                   = 2
    ingestion-identity       = var.use_aws ? local.ingestor_server_identity.aws-iam-entity : null
    ingestion-bucket         = local.ingestion_bucket_url
    peer-validation-identity = var.use_aws ? aws_iam_role.bucket_role[0].arn : null
    peer-validation-bucket   = local.peer_validation_bucket_url
    batch-signing-public-keys = {
      (module.kubernetes.batch_signing_key) = {
        public-key = ""
        expiration = ""
      }
    }
    packet-encryption-keys = {
      (var.packet_decryption_key_kubernetes_secret) = {
        certificate-signing-request = ""
      }
    }
    } : {
    format                   = 1
    ingestion-identity       = var.use_aws ? local.ingestor_server_identity.aws-iam-entity : null
    ingestion-bucket         = local.ingestion_bucket_url
    peer-validation-identity = null
    peer-validation-bucket   = local.peer_validation_bucket_url
    batch-signing-public-keys = {
      (module.kubernetes.batch_signing_key) = {
        public-key = ""
        expiration = ""
      }
    }
    packet-encryption-keys = {
      (var.packet_decryption_key_kubernetes_secret) = {
        certificate-signing-request = ""
      }
    }
  }
}
