variable "name" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "kms_key" {
  type = string
}

variable "bucket_reader" {
  type = string
}

variable "bucket_writer" {
  type = string
}

resource "google_storage_bucket" "bucket" {
  name     = var.name
  location = var.gcp_region
  # Force deletion of bucket contents on bucket destroy. Bucket contents would
  # be re-created by a subsequent deploy so no reason to keep them around.
  force_destroy               = true
  uniform_bucket_level_access = true
  # Automatically purge data after 7 days
  # https://cloud.google.com/storage/docs/lifecycle
  lifecycle_rule {
    action {
      type = "Delete"
    }
    condition {
      age = 1
    }
  }
  # Encrypt bucket contents at rest using KMS key
  # https://cloud.google.com/storage/docs/encryption/customer-managed-keys
  encryption {
    default_kms_key_name = var.kms_key
  }
}

resource "google_storage_bucket_iam_binding" "bucket_writer" {
  bucket = google_storage_bucket.bucket.name
  role   = "roles/storage.legacyBucketWriter"
  members = [
    "serviceAccount:${var.bucket_writer}"
  ]
}

resource "google_storage_bucket_iam_binding" "bucket_reader" {
  bucket = google_storage_bucket.bucket.name
  role   = "roles/storage.objectViewer"
  members = [
    "serviceAccount:${var.bucket_reader}"
  ]
}
