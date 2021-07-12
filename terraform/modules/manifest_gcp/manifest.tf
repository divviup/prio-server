variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "global_manifest_content" {
  type = string
}

# Make a bucket where we will store global and specific manifests and from which
# peers can fetch them.
# https://cloud.google.com/cdn/docs/setting-up-cdn-with-bucket
resource "google_storage_bucket" "manifests" {
  name     = "prio-${var.environment}-manifests"
  location = var.gcp_region
  # Force deletion of bucket contents on bucket destroy. Bucket contents would
  # be re-created by a subsequent deploy so no reason to keep them around.
  force_destroy = true
  # Disable per-object ACLs. Everything we put in here is meant to be world-
  # readable and this is also in line with Google's recommendation:
  # https://cloud.google.com/storage/docs/uniform-bucket-level-access
  uniform_bucket_level_access = true
}

# With uniform bucket level access, we must use IAM permissions as ACLs are
# ignored.
# https://cloud.google.com/storage/docs/uniform-bucket-level-access
resource "google_storage_bucket_iam_binding" "public_read" {
  bucket = google_storage_bucket.manifests.name
  # We want to allow unauthenticated reads of manifests. This also allows
  # listing the bucket, which could allow an attacker to enumerate the PHAs
  # that we have deployed support for. On the other hand, the attacker could
  # also get that information by reading the tfvars files on GitHub.
  # https://cloud.google.com/storage/docs/access-control/lists#predefined-acl
  role = "roles/storage.objectViewer"
  members = [
    "allUsers"
  ]
}

# Puts this data share processor's global manifest into the bucket.
resource "google_storage_bucket_object" "global_manifest" {
  name          = "global-manifest.json"
  bucket        = google_storage_bucket.manifests.name
  content_type  = "application/json"
  cache_control = "no-cache"
  content       = var.global_manifest_content
}

output "bucket" {
  value = google_storage_bucket.manifests.name
}

output "base_url" {
  value = "storage.googleapis.com/${google_storage_bucket.manifests.name}"
}
