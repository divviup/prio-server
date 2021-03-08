variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "manifest_bucket" {
  type = string
}

variable "sum_part_bucket_writer_email" {
  type = string
}

# For our purposes, a fake portal server is simply a bucket where we can write
# sum parts, as well as a correctly formed global manifest advertising that
# bucket's name.
resource "google_storage_bucket" "sum_part_output" {
  name     = "prio-${var.environment}-sum-part-output"
  location = var.gcp_region
  # Force deletion of bucket contents on bucket destroy. Bucket contents would
  # be re-created by a subsequent deploy so no reason to keep them around.
  force_destroy = true
  # Disable per-object ACLs. Everything we put in here is meant to be world-
  # readable and this is also in line with Google's recommendation:
  # https://cloud.google.com/storage/docs/uniform-bucket-level-access
  uniform_bucket_level_access = true
}

# Enable the sum part bucket writer GCP SA to write output
resource "google_storage_bucket_iam_binding" "write_sum_parts" {
  bucket = google_storage_bucket.sum_part_output.name
  # Allow ourselves to write to sum part outputs
  role = "roles/storage.objectAdmin"
  members = [
    "serviceAccount:${var.sum_part_bucket_writer_email}"
  ]
}

# Create a portal server global manifest and advertise it from our manifest
# bucket. Note that the manifest bucket name and the relative path in this
# resource's name field must match the portal_server_manifest_base_url value in
# this env's .tfvars!
resource "google_storage_bucket_object" "portal_server_global_manifest" {
  name         = "portal-server/global-manifest.json"
  bucket       = var.manifest_bucket
  content_type = "application/json"
  content = jsonencode({
    format = 1
    # We're cheating here by listing the same bucket twice, but the other env
    # will consult a totally different portal server global manifest.
    facilitator-sum-part-bucket = "gs://${google_storage_bucket.sum_part_output.name}"
    pha-sum-part-bucket         = "gs://${google_storage_bucket.sum_part_output.name}"
  })
}
