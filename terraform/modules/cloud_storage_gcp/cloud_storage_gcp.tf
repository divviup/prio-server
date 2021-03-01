variable "gcp_region" {
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

variable "peer_validation_bucket_name" {
  type = string
}

variable "peer_validation_bucket_writer" {
  type = string
}

variable "kms_key" {
  type = string
}

locals {
  bucket_parameters = {
    ingestion = {
      name   = var.ingestion_bucket_name
      writer = var.ingestion_bucket_writer
      reader = var.ingestion_bucket_reader
    }
    local_peer_validation = {
      name   = var.peer_validation_bucket_name
      writer = var.peer_validation_bucket_writer
      reader = var.ingestion_bucket_reader
    }
  }
}

module "bucket" {
  for_each = toset(["ingestion", "local_peer_validation"])
  source   = "./bucket"

  name          = local.bucket_parameters[each.value].name
  bucket_reader = local.bucket_parameters[each.value].reader
  bucket_writer = local.bucket_parameters[each.value].writer
  kms_key       = var.kms_key
  gcp_region    = var.gcp_region
}
