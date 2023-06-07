variable "gcp_region" {
  type = string
}

variable "bucket_reader" {
  type = string
}

variable "ingestion_bucket_name" {
  type = string
}

variable "ingestion_bucket_writer" {
  type = string
}

variable "peer_validation_bucket_name" {
  type = string
}

variable "peer_validation_bucket_writer" {
  type = string
}

variable "own_validation_bucket_name" {
  type = string
}

variable "own_validation_bucket_writer" {
  type = string
}

variable "kms_keyring" {
  type = string
}

variable "resource_prefix" {
  type = string
}

locals {
  bucket_parameters = {
    ingestion = {
      name   = var.ingestion_bucket_name
      writer = var.ingestion_bucket_writer
    }
    local_peer_validation = {
      name   = var.peer_validation_bucket_name
      writer = var.peer_validation_bucket_writer
    }
    own_validation = {
      name   = var.own_validation_bucket_name
      writer = var.own_validation_bucket_writer
    }
  }
}

# KMS key used to encrypt GCS bucket contents at rest. We create one per data
# share processor, enabling us to cryptographically destroy one instance's
# storage without disrupting any others.
resource "google_kms_crypto_key" "bucket_encryption" {
  name     = "${var.resource_prefix}-bucket-encryption-key"
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

module "bucket" {
  for_each = toset(["ingestion", "local_peer_validation", "own_validation"])
  source   = "./bucket"

  name          = local.bucket_parameters[each.value].name
  bucket_reader = var.bucket_reader
  bucket_writer = local.bucket_parameters[each.value].writer
  kms_key       = google_kms_crypto_key.bucket_encryption.id
  gcp_region    = var.gcp_region

  grant_write_permissions = each.value != "ingestion"

  # Ensure the GCS service account exists and has permission to use the KMS key
  # before creating buckets.
  depends_on = [google_kms_crypto_key_iam_binding.bucket_encryption_key]
}
