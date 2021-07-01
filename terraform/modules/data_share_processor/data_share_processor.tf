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
  resource_prefix = "prio-${var.environment}-${var.data_share_processor_name}"

  peer_share_processor_server_identity           = jsondecode(data.http.peer_share_processor_global_manifest.body).server-identity
  peer_share_processor_aws_account_id            = local.peer_share_processor_server_identity.aws-account-id
  peer_share_processor_gcp_service_account_email = local.peer_share_processor_server_identity.gcp-service-account-email

  # There are three cases for who is accessing this data share processor's
  # storage buckets, listed in the order we check for them:
  #
  # 1 - This is a test environment that does *not* create fake ingestors and
  #     whose storage is partially in s3. use_aws will be set to true for this.
  # 2 - This is either a test, or non-test environment whose storage is
  #     exclusively in GCS and for an ingestor that advertises a GCP service account email.
  #
  # The case we no longer support is a non-test environment for an ingestor that
  # advertises an AWS role.
  bucket_access_identities = var.use_aws ? (
    {
      # In the test setup, the peer env hosts the sample-makers, so allow
      # entities in the peer's AWS account to write to our ingeston bucket
      ingestion_bucket_writer = local.ingestor_server_identity.aws-iam-entity
      # We must assume this AWS role to read our ingestion bucket
      ingestion_bucket_reader = aws_iam_role.bucket_role.arn
      # The identity that GKE jobs should assume or impersonate to access the
      # ingestion bucket.
      ingestion_identity = aws_iam_role.bucket_role.arn
      # We impersonate this GCP SA to write to the peer's validation bucket
      remote_validation_bucket_writer = var.remote_bucket_writer_gcp_service_account_email
      # We permit entities in the peer's AWS account to write their validations
      # to our bucket
      peer_validation_bucket_writer = local.peer_share_processor_aws_account_id
      # We assume this AWS role to read the bucket to which peers wrote their
      # validations
      peer_validation_bucket_reader = aws_iam_role.bucket_role.arn
      # The identity that GKE jobs should assume or impersonate to access the
      # local peer validation bucket
      peer_validation_identity = aws_iam_role.bucket_role.arn
    }
    ) : (
    {
      # Ingestors always act as a GCP service account, to which we grant write
      # permissions.
      ingestion_bucket_writer = local.ingestor_server_identity.gcp-service-account-email
      # No special auth is needed to read from the ingestion bucket in GCS
      ingestion_bucket_reader = module.kubernetes.service_account_email
      # The identity that GKE jobs should assume or impersonate to access the
      # ingestion bucket.
      ingestion_identity = ""
      # We assume this AWS IAM role to write our validations to the peer DSP's
      # bucket
      remote_validation_bucket_writer = aws_iam_role.bucket_role.arn
      # We permit the peer's GCP service account to write their validations to
      # our bucket
      peer_validation_bucket_writer = local.peer_share_processor_gcp_service_account_email
      # No special auth is needed to read from the validation bucket in GCS
      peer_validation_bucket_reader = module.kubernetes.service_account_email
      # The identity that GKE jobs should assume or impersonate to access the
      # local peer validation bucket
      peer_validation_identity = ""
    }
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
}

data "aws_caller_identity" "current" {}

# This is the role in AWS we use to access S3 buckets. Generally that means
# buckets owned by peers, but in some test deployments which use S3, this role
# also governs access to those S3 buckets. It is configured to allow access to
# the GCP service account for this data share processor via Web Identity
# Federation
# https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_oidc.html
resource "aws_iam_role" "bucket_role" {
  name = "${local.resource_prefix}-bucket-role"
  # Since azp is set in the auth token Google generates, we must check oaud in
  # the role assumption policy, and the value must match what we request when
  # requesting tokens from the GKE metadata service in
  # GkeMetadataServiceIdentityTokenProvider::identity_token
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
          "accounts.google.com:sub": "${module.kubernetes.service_account_unique_id}",
          "accounts.google.com:oaud": "sts.amazonaws.com/gke-identity-federation"
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
  name = "${local.resource_prefix}-bucket-role-policy"
  role = aws_iam_role.bucket_role.id

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

# KMS key used to encrypt GCS bucket contents at rest. We create one per data
# share processor, enabling us to cryptographically destroy one instance's
# storage without disrupting any others.
resource "google_kms_crypto_key" "bucket_encryption" {
  name     = "${local.resource_prefix}-bucket-encryption-key"
  key_ring = var.kms_keyring
  purpose  = "ENCRYPT_DECRYPT"
  # Rotate database encryption key every 90 days. This won't re-encrypt existing
  # content, but new data will be encrypted under the new key.
  rotation_period = "7776000s"
}

# Permit the GCS service account to use the KMS key
# https://registry.terraform.io/providers/hashicorp/google/latest/docs/data-sources/storage_project_service_account
data "google_storage_project_service_account" "gcs_account" {}

resource "google_kms_crypto_key_iam_binding" "bucket_encryption_key" {
  crypto_key_id = google_kms_crypto_key.bucket_encryption.id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  members = [
    "serviceAccount:${data.google_storage_project_service_account.gcs_account.email_address}"
  ]
}

# For test purposes, we support creating the ingestion and peer validation
# buckets in AWS S3, even though all ISRG storage is in Google Cloud Storage. We
# only create the ingestion bucket and peer validation bucket there, so that we
# can exercise the parameter exchange and authentication flows.
# https://github.com/abetterinternet/prio-server/issues/68
module "cloud_storage_aws" {
  count                         = var.use_aws ? 1 : 0
  source                        = "../../modules/cloud_storage_aws"
  environment                   = var.environment
  ingestion_bucket_name         = local.ingestion_bucket_name
  ingestion_bucket_writer       = local.bucket_access_identities.ingestion_bucket_writer
  ingestion_bucket_reader       = local.bucket_access_identities.ingestion_bucket_reader
  peer_validation_bucket_name   = local.peer_validation_bucket_name
  peer_validation_bucket_writer = local.bucket_access_identities.peer_validation_bucket_writer
  peer_validation_bucket_reader = local.bucket_access_identities.peer_validation_bucket_reader
}

# In real ISRG deployments, all of our storage is in GCS
module "cloud_storage_gcp" {
  count                         = var.use_aws ? 0 : 1
  source                        = "../../modules/cloud_storage_gcp"
  gcp_region                    = var.gcp_region
  kms_key                       = google_kms_crypto_key.bucket_encryption.id
  ingestion_bucket_name         = local.ingestion_bucket_name
  ingestion_bucket_writer       = local.bucket_access_identities.ingestion_bucket_writer
  ingestion_bucket_reader       = local.bucket_access_identities.ingestion_bucket_reader
  peer_validation_bucket_name   = local.peer_validation_bucket_name
  peer_validation_bucket_writer = local.bucket_access_identities.peer_validation_bucket_writer
  # Ensure the GCS service account exists and has permission to use the KMS key
  # before creating buckets.
  depends_on = [google_kms_crypto_key_iam_binding.bucket_encryption_key]
}

# Besides the validation bucket owned by the peer data share processor, we write
# validation batches into a bucket we control so that we can be certain they
# be available when we perform the aggregation step.
module "bucket" {
  source        = "../../modules/cloud_storage_gcp/bucket"
  name          = "${local.resource_prefix}-own-validation"
  bucket_reader = module.kubernetes.service_account_email
  bucket_writer = module.kubernetes.service_account_email
  kms_key       = google_kms_crypto_key.bucket_encryption.id
  gcp_region    = var.gcp_region
  # Ensure the GCS service account exists and has permission to use the KMS key
  # before creating buckets.
  depends_on = [google_kms_crypto_key_iam_binding.bucket_encryption_key]
}

module "pubsub" {
  for_each                   = toset(["intake", "aggregate"])
  source                     = "../../modules/pubsub"
  environment                = var.environment
  data_share_processor_name  = var.data_share_processor_name
  publisher_service_account  = module.kubernetes.service_account_email
  subscriber_service_account = module.kubernetes.service_account_email
  task                       = each.key
}

module "kubernetes" {
  source                                  = "../../modules/kubernetes/"
  data_share_processor_name               = var.data_share_processor_name
  ingestor                                = var.ingestor
  environment                             = var.environment
  kubernetes_namespace                    = var.kubernetes_namespace
  ingestion_bucket                        = local.ingestion_bucket_url
  ingestion_bucket_identity               = local.bucket_access_identities.ingestion_identity
  ingestor_manifest_base_url              = var.ingestor_manifest_base_url
  packet_decryption_key_kubernetes_secret = var.packet_decryption_key_kubernetes_secret
  peer_manifest_base_url                  = var.peer_share_processor_manifest_base_url
  remote_peer_validation_bucket_identity  = local.bucket_access_identities.remote_validation_bucket_writer
  peer_validation_bucket                  = local.peer_validation_bucket_url
  peer_validation_bucket_identity         = local.bucket_access_identities.peer_validation_identity
  own_validation_bucket                   = "gs://${local.own_validation_bucket_name}"
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
  intake_queue                            = module.pubsub["intake"].queue
  aggregate_queue                         = module.pubsub["aggregate"].queue
  intake_worker_count                     = var.intake_worker_count
  aggregate_worker_count                  = var.aggregate_worker_count
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

output "service_account_email" {
  value = module.kubernetes.service_account_email
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
