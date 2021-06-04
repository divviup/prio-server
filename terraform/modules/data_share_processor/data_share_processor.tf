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

variable "oidc_provider" {
  type = object({
    arn = string
    url = string
  })
  default = {
    arn = ""
    url = ""
  }
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

  peer_share_processor_server_identity           = jsondecode(data.http.peer_share_processor_global_manifest.body).server-identity
  peer_share_processor_aws_account_id            = local.peer_share_processor_server_identity.aws-account-id
  peer_share_processor_gcp_service_account_email = local.peer_share_processor_server_identity.gcp-service-account-email

  # When running in AWS, we impersonate a GCP service account to write to the
  # peer's validation bucket in GCS. When in GKE, we assume an AWS role to write
  # to their S3 bucket.
  remote_validation_bucket_writer = var.use_aws ? (
    var.remote_bucket_writer_gcp_service_account_email
    ) : (
    aws_iam_role.bucket_role[0].arn
  )
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
  intake_queue = var.use_aws ? {
    topic        = module.sns_sqs["intake"].topic
    subscription = module.sns_sqs["intake"].subscription
    } : {
    topic        = module.pubsub["intake"].topic
    subscription = module.pubsub["intake"].subscription
  }

  aggregate_queue = var.use_aws ? {
    topic        = module.sns_sqs["aggregate"].topic
    subscription = module.sns_sqs["aggregate"].subscription
    } : {
    topic        = module.pubsub["aggregate"].topic
    subscription = module.pubsub["aggregate"].subscription
  }
}

data "aws_caller_identity" "current" {}

# For data share processors that run in GKE, this AWS IAM role is assumed to
# access peer validation buckets in S3. It is configured to allow access to the
# GCP service account for this data share processor via Web Identity Federation
# https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_oidc.html
resource "aws_iam_role" "bucket_role" {
  count = var.use_aws ? 0 : 1

  name = "${local.resource_prefix}-bucket-role"
  # Since azp is set in the auth token Google generates, we must check oaud in
  # the role assumption policy, and the value must match what we request when
  # requesting tokens from the GKE metadata service in
  # S3Transport::new_with_client
  # https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_iam-condition-keys.html
  assume_role_policy = <<ROLE
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "Federated": "accounts.google.com"
      },
      "Action": "sts:AssumeRoleWithWebIdentity",
      "Condition": {
        "StringEquals": {
          "accounts.google.com:sub": "${module.kubernetes.gcp_service_account_unique_id}",
          "accounts.google.com:oaud": "sts.amazonaws.com/${data.aws_caller_identity.current.account_id}"
        }
      }
    }
  ]
}
ROLE

  tags = {
    environment = "prio-${var.environment}"
  }
}

# The bucket_role defined above is created in our AWS account and used to
# access S3 buckets in a peer's AWS account. Besides _their_ bucket having a
# policy that grants us access, we need to make sure _our_ role allows use of S3
# API, hence this policy. Note it is not the same as the role assumption policy
# defined inline in aws_iam_role.bucket_role.
# TODO: Make this policy as restrictive as possible:
# https://github.com/abetterinternet/prio-server/issues/140
resource "aws_iam_role_policy" "bucket_role_policy" {
  count = var.use_aws ? 0 : 1

  name = "${local.resource_prefix}-bucket-role-policy"
  role = aws_iam_role.bucket_role[0].id

  policy = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Action": [
        "s3:*"
      ],
      "Effect": "Allow",
      "Resource": "*"
    }
  ]
}
POLICY
}

# If our ingestion bucket is in S3 and this data share processor's ingestion
# server uses a Google identity, then we must create an AWS IAM role for it to
# assume.
resource "aws_iam_role" "ingestion_bucket_writer" {
  count = (var.use_aws && local.ingestor_gcp_service_account_id != "") ? 1 : 0

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

# For test purposes, we support creating the ingestion and peer validation
# buckets in AWS S3, even though all ISRG storage is in Google Cloud Storage. We
# only create the ingestion bucket and peer validation bucket there, so that we
# can exercise the parameter exchange and authentication flows.
# https://github.com/abetterinternet/prio-server/issues/68
module "cloud_storage_aws" {
  count  = var.use_aws ? 1 : 0
  source = "../../modules/cloud_storage_aws"

  resource_prefix       = local.resource_prefix
  bucket_reader         = module.kubernetes.aws_iam_role
  ingestion_bucket_name = local.ingestion_bucket_name
  # If the ingestor has a GCP service account, grant write access to
  # aws_iam_role.ingestion_bucket_writer, the IAM role configured to allow
  # assumption by the GCP SA. If the ingestor has an AWS IAM role, just grant
  # permission to it.
  ingestion_bucket_writer = local.ingestor_gcp_service_account_id != "" ? (
    aws_iam_role.ingestion_bucket_writer[0].arn
    ) : (
    local.ingestor_server_identity.aws-iam-entity
  )
  peer_validation_bucket_name   = local.peer_validation_bucket_name
  peer_validation_bucket_writer = local.peer_share_processor_aws_account_id
  own_validation_bucket_name    = "${local.resource_prefix}-own-validation"
  own_validation_bucket_writer  = module.kubernetes.aws_iam_role
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
  for_each = toset(var.use_aws ? [] : ["intake", "aggregate"])
  source   = "../../modules/pubsub"

  environment                = var.environment
  data_share_processor_name  = var.data_share_processor_name
  publisher_service_account  = module.kubernetes.gcp_service_account_email
  subscriber_service_account = module.kubernetes.gcp_service_account_email
  task                       = each.key
}

module "sns_sqs" {
  for_each = toset(var.use_aws ? ["intake", "aggregate"] : [])
  source   = "../../modules/sns_sqs"

  environment               = var.environment
  data_share_processor_name = var.data_share_processor_name
  publisher_iam_role        = module.kubernetes.aws_iam_role
  subscriber_iam_role       = module.kubernetes.aws_iam_role
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
  oidc_provider                           = var.oidc_provider
  use_aws                                 = var.use_aws
  aws_region                              = var.aws_region
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

output "specific_manifest" {
  value = {
    format                 = 1
    ingestion-identity     = var.use_aws ? local.ingestor_server_identity.aws-iam-entity : null
    ingestion-bucket       = local.ingestion_bucket_url,
    peer-validation-bucket = local.peer_validation_bucket_url,
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
