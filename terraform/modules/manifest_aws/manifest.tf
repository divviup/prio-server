variable "environment" {
  type = string
}

variable "global_manifest_content" {
  type = string
}

# S3 bucket from which we serve global and specific manifests. Unlike the GCP
# case, we don't set up Cloudfront with an ISRG owned domain. Peers can trust us
# based on our S3 bucket name just as well as they could based on a domain name.
resource "aws_s3_bucket" "manifests" {
  bucket = "isrg-prio-${var.environment}-manifests"
  # Allow unauthenticated reads of manifests
  acl = "public-read"
  # Destroy bucket contents when destroying the bucket
  force_destroy = true
}

resource "aws_s3_bucket_object" "global_manifest" {
  bucket        = aws_s3_bucket.manifests.bucket
  key           = "global-manifest.json"
  content       = var.global_manifest_content
  acl           = "public-read"
  cache_control = "no-cache"
  content_type  = "application/json"
}

output "bucket" {
  value = aws_s3_bucket.manifests.bucket
}

output "base_url" {
  value = aws_s3_bucket.manifests.bucket_domain_name
}
