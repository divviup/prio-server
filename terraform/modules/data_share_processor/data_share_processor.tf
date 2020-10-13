variable "data_share_processor_name" {
  type = string
}

variable "environment" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "ingestor_aws_role_arn" {
  type = string
}

variable "ingestor_google_service_account_id" {
  type = string
}

variable "peer_share_processor_aws_account_id" {
  type = string
}

locals {
  resource_prefix                  = "${var.environment}-${var.data_share_processor_name}"
  ingestion_bucket_writer_role_arn = var.ingestor_google_service_account_id != "" ? aws_iam_role.ingestor_bucket_writer_role[0].arn : var.ingestor_aws_role_arn
  ingestion_bucket_name            = "prio-${local.resource_prefix}-ingestion"
  validation_bucket_name           = "prio-${local.resource_prefix}-validation"
  sum_part_bucket_name             = "prio-${local.resource_prefix}-sum-part"
}

# This is the role in AWS we use to construct policy on the S3 buckets. It is
# configured to allow access to the GCP service account for this data share
# processor via Web Identity Federation
# https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_oidc.html
resource "aws_iam_role" "bucket_owner_role" {
  name = "prio-${local.resource_prefix}-bucket-role"
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

# If the ingestor authenticates using a GCP service account, this is the role in
# AWS that their service account assumes. Note the "count" parameter in the
# block, which seems to be the Terraform convention to conditionally create
# resources (c.f. lots of StackOverflow questions and GitHub issues).
resource "aws_iam_role" "ingestor_bucket_writer_role" {
  count              = var.ingestor_google_service_account_id != "" ? 1 : 0
  name               = "prio-${local.resource_prefix}-bucket-writer"
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
          "accounts.google.com:sub": "${var.ingestor_google_service_account_id}"
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

# The ingestion bucket for this data share processor. This one is different from
# the other two in that we must grant write access to the ingestor but no access
# to the peer share processor.
resource "aws_s3_bucket" "ingestion_bucket" {
  bucket = local.ingestion_bucket_name
  # Force deletion of bucket contents on bucket destroy.
  force_destroy = true
  policy        = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${local.ingestion_bucket_writer_role_arn}"
      },
      "Action": [
        "s3:AbortMultipartUpload",
        "s3:PutObject",
        "s3:ListMultipartUploadParts",
        "s3:ListBucketMultipartUploads"
      ],
      "Resource": [
        "arn:aws:s3:::${local.ingestion_bucket_name}/*",
        "arn:aws:s3:::${local.ingestion_bucket_name}"
      ]
    },
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${aws_iam_role.bucket_owner_role.arn}"
      },
      "Action": [
        "s3:GetObject"
      ],
      "Resource": [
        "arn:aws:s3:::${local.ingestion_bucket_name}/*",
        "arn:aws:s3:::${local.ingestion_bucket_name}"
      ]
    }
  ]
}
POLICY
  tags = {
    environment = var.environment
  }
}

# The validation and sum part buckets for this data share processor, configured
# to permit the peer share processor to read it. The policy grants read
# permissions to any role or user in the peer data share processor's AWS account
# because Amazon S3 won't let you define policies in terms of roles that don't
# exist. In any case, since we have no control over or insight into role
# assumption policies in the other account, we gain nothing by specifying
# anything beyond the AWS account.
resource "aws_s3_bucket" "other_buckets" {
  for_each = toset([
    local.validation_bucket_name, local.sum_part_bucket_name
  ])
  bucket = each.key
  # Force deletion of bucket contents on bucket destroy.
  force_destroy = true
  policy        = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${aws_iam_role.bucket_owner_role.arn}"
      },
      "Action": [
        "s3:AbortMultipartUpload",
        "s3:PutObject",
        "s3:ListMultipartUploadParts",
        "s3:ListBucketMultipartUploads"
      ],
      "Resource": [
        "arn:aws:s3:::${each.key}/*",
        "arn:aws:s3:::${each.key}"
      ]
    },
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": [
          "${aws_iam_role.bucket_owner_role.arn}",
          "${var.peer_share_processor_aws_account_id}"
        ]
      },
      "Action": [
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
  data_share_processor_name = var.data_share_processor_name
  gcp_project               = var.gcp_project
  environment               = var.environment
  ingestion_bucket          = "${aws_s3_bucket.ingestion_bucket.region}/${aws_s3_bucket.ingestion_bucket.bucket}"
  ingestion_bucket_role     = aws_iam_role.bucket_owner_role.arn
}

output "data_share_processor_name" {
  value = var.data_share_processor_name
}

output "specific_manifest" {
  value = {
    format            = 0
    ingestion-bucket  = "${aws_s3_bucket.ingestion_bucket.region}/${aws_s3_bucket.ingestion_bucket.bucket}",
    validation-bucket = "${aws_s3_bucket.other_buckets[local.validation_bucket_name].region}/${aws_s3_bucket.other_buckets[local.validation_bucket_name].bucket}",
    sum-part-bucket   = "${aws_s3_bucket.other_buckets[local.sum_part_bucket_name].region}/${aws_s3_bucket.other_buckets[local.sum_part_bucket_name].bucket}",
    batch-signing-keys = {
      (module.kubernetes.batch_signing_key) = {
        public-key = ""
        expiration = ""
      }
    }
    packet-encryption-certificates = {
      (module.kubernetes.ingestion_packet_decryption_key) = ""
    }
  }
}
