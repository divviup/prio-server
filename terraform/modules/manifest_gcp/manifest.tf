variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "managed_dns_zone" {
  type = map(string)
}

variable "global_manifest_content" {
  type = string
}

resource "google_project_service" "compute" {
  service = "compute.googleapis.com"
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

locals {
  domain_name = "${var.environment}.${data.google_dns_managed_zone.manifests.dns_name}"
}

# Now we configure an external HTTPS load balancer backed by the bucket.
resource "google_compute_managed_ssl_certificate" "manifests" {
  # Managed SSL certificates are GA but the Terraform provider still restricts
  # them to the beta variant.
  provider = google-beta

  name = "prio-${var.environment}-manifests"
  managed {
    domains = [local.domain_name]
  }

  depends_on = [google_project_service.compute]
}

# We expect a managed DNS zone in which we can create subdomains for a given
# env's manifest endpoint to already exist, outside of this Terraform module.
data "google_dns_managed_zone" "manifests" {
  name = var.managed_dns_zone.name
  # The managed zone is not necessarily in the same GCP project as this env, so
  # we pass the project all the way from tfvars to here.
  project = var.managed_dns_zone.gcp_project
}

# Create an A record from which this env's manifests will be served.
resource "google_dns_record_set" "manifests" {
  project      = var.managed_dns_zone.gcp_project
  name         = local.domain_name
  managed_zone = data.google_dns_managed_zone.manifests.name
  type         = "A"
  ttl          = 300
  rrdatas      = [google_compute_global_address.manifests.address]
}

# Reserve an external IP address for the load balancer.
# https://cloud.google.com/cdn/docs/setting-up-cdn-with-bucket#ip-address
resource "google_compute_global_address" "manifests" {
  name = "prio-${var.environment}-manifests"

  depends_on = [google_project_service.compute]
}

resource "google_compute_backend_bucket" "manifests" {
  name        = "prio-${var.environment}-manifest-backend"
  bucket_name = google_storage_bucket.manifests.name
  enable_cdn  = true

  depends_on = [google_project_service.compute]
}

resource "google_compute_url_map" "manifests" {
  name            = "prio-${var.environment}-manifests"
  default_service = google_compute_backend_bucket.manifests.id
}

resource "google_compute_target_https_proxy" "manifests" {
  name             = "prio-${var.environment}-manifests"
  url_map          = google_compute_url_map.manifests.id
  ssl_certificates = [google_compute_managed_ssl_certificate.manifests.id]
}

resource "google_compute_global_forwarding_rule" "manifests" {
  name       = "prio-${var.environment}-manifests"
  ip_address = google_compute_global_address.manifests.address
  port_range = "443"
  target     = google_compute_target_https_proxy.manifests.id
}

output "bucket" {
  value = google_storage_bucket.manifests.name
}

output "base_url" {
  # local.domain_name is a fully qualified DNS name, ending in '.'
  value = trimsuffix(local.domain_name, ".")
}
