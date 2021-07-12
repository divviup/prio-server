variable "resource_prefix" {
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

resource "aws_kms_key" "bucket_encryption" {
  description = "Encryption at rest for S3 buckets in ${var.resource_prefix} data share processor"
  key_usage   = "ENCRYPT_DECRYPT"

  tags = {
    Name = "${var.resource_prefix}-bucket-encryption"
  }
}

resource "aws_s3_bucket" "buckets" {
  for_each = toset(["ingestion", "local_peer_validation", "own_validation"])

  bucket = local.bucket_parameters[each.value].name
  # Force deletion of bucket contents on bucket destroy
  force_destroy = true
  # Delete objects 7 days after creation
  lifecycle_rule {
    enabled = true
    expiration {
      days = 7
    }
  }
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          AWS = local.bucket_parameters[each.value].writer
        }
        Action = [
          "s3:AbortMultipartUpload",
          "s3:PutObject",
          "s3:ListMultipartUploadParts",
          "s3:ListBucketMultipartUploads",
        ]
        Resource = [
          "arn:aws:s3:::${local.bucket_parameters[each.value].name}/*",
          "arn:aws:s3:::${local.bucket_parameters[each.value].name}"
        ]
      },
      {
        Effect = "Allow"
        Principal = {
          AWS = var.bucket_reader
        }
        Action = [
          "s3:GetObject",
          "s3:ListBucket",
        ]
        Resource = [
          "arn:aws:s3:::${local.bucket_parameters[each.value].name}/*",
          "arn:aws:s3:::${local.bucket_parameters[each.value].name}"
        ]
      }
    ]
  })
}
