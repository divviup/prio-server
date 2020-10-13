variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "domain" {
  type = string
}

data "aws_caller_identity" "current" {}

# Make a bucket where we will store global and specific manifests and from which
# peers can fetch them.
# https://cloud.google.com/cdn/docs/setting-up-cdn-with-bucket
resource "google_storage_bucket" "manifests" {
  provider = google-beta
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
  provider     = google-beta
  name         = "global-manifest.json"
  bucket       = google_storage_bucket.manifests.name
  content_type = "application/json"
  content      = <<MANIFEST
{
  "format": 0,
  "aws-account-id": "${data.aws_caller_identity.current.account_id}"
}
MANIFEST
}

# Now we configure an external HTTPS load balancer backed by the bucket.
resource "google_compute_managed_ssl_certificate" "manifests" {
  provider = google-beta
  name     = "prio-${var.environment}-manifests"
  managed {
    domains = [var.domain]
  }
}

# Reserve an external IP address for the load balancer.
# https://cloud.google.com/cdn/docs/setting-up-cdn-with-bucket#ip-address
# TODO(timg): we should have Terraform configure DNS for the manifest domain so
# we can point it at this IP address, without which the managed certificate will
# not work.
resource "google_compute_global_address" "manifests" {
  provider = google-beta
  name     = "prio-${var.environment}-manifests"
}

resource "google_compute_backend_bucket" "manifests" {
  provider    = google-beta
  name        = "prio-${var.environment}-manifest-backend"
  bucket_name = google_storage_bucket.manifests.name
  enable_cdn  = true
}

resource "google_compute_url_map" "manifests" {
  provider        = google-beta
  name            = "prio-${var.environment}-manifests"
  default_service = google_compute_backend_bucket.manifests.id
}

resource "google_compute_target_https_proxy" "manifests" {
  provider         = google-beta
  name             = "prio-${var.environment}-manifests"
  url_map          = google_compute_url_map.manifests.id
  ssl_certificates = [google_compute_managed_ssl_certificate.manifests.id]
}

resource "google_compute_global_forwarding_rule" "manifests" {
  provider   = google-beta
  name       = "prio-${var.environment}-manifests"
  ip_address = google_compute_global_address.manifests.address
  port_range = "443"
  target     = google_compute_target_https_proxy.manifests.id
}

output "bucket" {
  value = google_storage_bucket.manifests.name
}
