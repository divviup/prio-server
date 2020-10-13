variable "peer_share_processor_name" {
  type = string
}

variable "environment" {
  type = string
}

variable "gcp_project" {
  type = string
}

# This is the role in AWS we use to construct policy on the S3 buckets. It is
# configured to allow access to the GCP service account for this data share
# processor via Web Identity Federation
# https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_oidc.html
resource "aws_iam_role" "bucket_role" {
  name = "prio-${var.environment}-${var.peer_share_processor_name}-bucket-role"
  # We currently use a single role per data share processor to gate read/write
  # access to all buckets. We could use audiences to define more roles for
  # read/write on each of the ingestion, validation and sum part buckets.
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
          "accounts.google.com:sub": "${module.kubernetes.service_account_unique_id}"
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

locals {
  ingestion_bucket_name  = "prio-${var.environment}-${var.peer_share_processor_name}-ingestion"
  validation_bucket_name = "prio-${var.environment}-${var.peer_share_processor_name}-validation"
  sum_part_bucket_name   = "prio-${var.environment}-${var.peer_share_processor_name}-sum-part"
}

resource "aws_s3_bucket" "buckets" {
  for_each = toset([
    local.ingestion_bucket_name, local.validation_bucket_name, local.sum_part_bucket_name
  ])
  bucket = each.key
  policy = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${aws_iam_role.bucket_role.arn}"
      },
      "Action": [
        "s3:AbortMultipartUpload",
        "s3:PutObject",
        "s3:ListMultipartUploadParts",
        "s3:ListBucketMultipartUploads",
        "s3:GetObject"
      ],
      "Resource": [
        "arn:aws:s3:::${each.key}/*",
        "arn:aws:s3:::${each.key}"
      ]
    }
  ]
}
POLICY
  tags = {
    environment = var.environment
  }
}

module "kubernetes" {
  source                    = "../../modules/kubernetes/"
  peer_share_processor_name = var.peer_share_processor_name
  gcp_project               = var.gcp_project
  environment               = var.environment
  ingestion_bucket          = "${aws_s3_bucket.buckets[local.ingestion_bucket_name].region}/${aws_s3_bucket.buckets[local.ingestion_bucket_name].bucket}"
  ingestion_bucket_role     = aws_iam_role.bucket_role.arn
}
