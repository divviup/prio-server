variable "environment" {
  type = string
}

variable "ingestion_bucket_name" {
  type = string
}

variable "ingestion_bucket_writer" {
  type = string
}

variable "ingestion_bucket_reader" {
  type = string
}

variable "local_peer_validation_bucket_name" {
  type = string
}

variable "local_peer_validation_bucket_writer" {
  type = string
}

variable "local_peer_validation_bucket_reader" {
  type = string
}

# The S3 ingestion bucket. It is configured to allow writes from the AWS
# account used by the ingestor and reads from the AWS IAM role assumed by this
# data share processor.
resource "aws_s3_bucket" "ingestion_bucket" {
  bucket = var.ingestion_bucket_name
  # Force deletion of bucket contents on bucket destroy.
  force_destroy = true
  policy        = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${var.ingestion_bucket_writer}"
      },
      "Action": [
        "s3:AbortMultipartUpload",
        "s3:PutObject",
        "s3:ListMultipartUploadParts",
        "s3:ListBucketMultipartUploads"
      ],
      "Resource": [
        "arn:aws:s3:::${var.ingestion_bucket_name}/*",
        "arn:aws:s3:::${var.ingestion_bucket_name}"
      ]
    },
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${var.ingestion_bucket_reader}"
      },
      "Action": [
        "s3:GetObject",
        "s3:ListBucket"
      ],
      "Resource": [
        "arn:aws:s3:::${var.ingestion_bucket_name}/*",
        "arn:aws:s3:::${var.ingestion_bucket_name}"
      ]
    }
  ]
}
POLICY

  tags = {
    environment = "prio-${var.environment}"
  }
}

# The peer validation bucket for this data share processor, configured to permit
# the peer share processor to write to it.
resource "aws_s3_bucket" "local_peer_validation_bucket" {
  bucket = var.local_peer_validation_bucket_name
  # Force deletion of bucket contents on bucket destroy.
  force_destroy = true
  policy        = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${var.local_peer_validation_bucket_writer}"
      },
      "Action": [
        "s3:AbortMultipartUpload",
        "s3:PutObject",
        "s3:ListMultipartUploadParts",
        "s3:ListBucketMultipartUploads"
      ],
      "Resource": [
        "arn:aws:s3:::${var.local_peer_validation_bucket_name}/*",
        "arn:aws:s3:::${var.local_peer_validation_bucket_name}"
      ]
    },
    {
      "Effect": "Allow",
      "Principal": {
        "AWS": "${var.local_peer_validation_bucket_reader}"
      },
      "Action": [
        "s3:GetObject",
        "s3:ListBucket"
      ],
      "Resource": [
        "arn:aws:s3:::${var.local_peer_validation_bucket_name}/*",
        "arn:aws:s3:::${var.local_peer_validation_bucket_name}"
      ]
    }
  ]
}
POLICY

  tags = {
    environment = "prio-${var.environment}"
  }
}
