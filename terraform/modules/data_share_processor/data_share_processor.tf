variable "ingestor" {
  type = string
}

variable "data_share_processor_name" {
  type = string
}

variable "environment" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "use_aws" {
  type = bool
}

variable "pure_gcp" {
  type = bool
}

variable "aws_region" {
  type = string
}

variable "kubernetes_namespace" {
  type = string
}

variable "packet_decryption_key_kubernetes_secret" {
  type = string
}

variable "certificate_domain" {
  type = string
}

variable "ingestor_manifest_base_url" {
  type = string
}

variable "peer_share_processor_manifest_base_url" {
  type = string
}

variable "remote_bucket_writer_gcp_service_account_email" {
  type = string
}

variable "portal_server_manifest_base_url" {
  type = string
}

variable "is_first" {
  type = bool
}

variable "intake_max_age" {
  type = string
}

variable "aggregation_period" {
  type = string
}

variable "aggregation_grace_period" {
  type = string
}

variable "kms_keyring" {
  type = string
}

variable "pushgateway" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "workflow_manager_image" {
  type = string
}

variable "workflow_manager_version" {
  type = string
}

variable "facilitator_image" {
  type = string
}

variable "facilitator_version" {
  type = string
}

variable "min_intake_worker_count" {
  type = number
}

variable "max_intake_worker_count" {
  type = number
}

variable "min_aggregate_worker_count" {
  type = number
}

variable "max_aggregate_worker_count" {
  type = number
}

variable "eks_oidc_provider" {
  type = object({
    arn = string
    url = string
  })
  default = {
    arn = ""
    url = ""
  }
}

variable "gcp_workload_identity_pool_provider" {
  type = string
}

variable "single_object_validation_batch_localities" {
  type        = set(string)
  description = <<DESCRIPTION
A set of localities where single-object validation batches are generated.
(Other localities use the "old" three-object format.) The special value "*"
indicates that all localities should generate single-object validation batches.
DESCRIPTION
}

variable "role_permissions_boundary_policy_arn" {
  type    = string
  default = null
}

variable "enable_heap_profiles" {
  type = bool
}

variable "profile_nfs_server" {
  type = string
}

locals {
  canned_ingestor_server_identities = {
    "ta-ta-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-ak-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-ca-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-co-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-ct-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-dc-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-hi-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-la-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-ma-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-md-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-mo-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-nm-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-nv-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-ut-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-va-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-wa-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "us-wi-apple" = {
      "aws-iam-entity"            = "arn:aws:iam::690017092588:role/ct_service"
      "gcp-service-account-email" = "eprodxx@applesystem.iam.gserviceaccount.com"
    }
    "ta-ta-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-ta-ta.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "109020912100641285657"
    }
    "us-ak-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-ak.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "111551694987680708475"
    }
    "us-ca-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-ca.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "103840253743419033664"
    }
    "us-co-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-co.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "110611915166111847455"
    }
    "us-ct-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-ct.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "116496586096763013722"
    }
    "us-dc-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-dc.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "107515049260634291601"
    }
    "us-hi-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-hi.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "105215147671385522170"
    }
    "us-la-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-la.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "113423669241607447504"
    }
    "us-ma-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-ma.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "105249126647703109150"
    }
    "us-md-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-md.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "112131106368700882469"
    }
    "us-mo-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-mo.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "103336767879594040242"
    }
    "us-nm-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-nm.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "103830287952597922112"
    }
    "us-nv-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-nv.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "109143840727213928841"
    }
    "us-ut-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-ut.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "108996594782665934675"
    }
    "us-va-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-va.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "117421313726844821980"
    }
    "us-wa-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-wa.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "116332471847097368455"
    }
    "us-wi-g-enpa" = {
      "gcp-service-account-email" = "dataflow-job-runner@enpa-ingestion-us-wi.iam.gserviceaccount.com"
      "gcp-service-account-id"    = "102144474753019266865"
    }
  }
  ingestor_server_identity        = local.canned_ingestor_server_identities[var.data_share_processor_name]
  ingestor_gcp_service_account_id = lookup(local.ingestor_server_identity, "gcp-service-account-id", "")
  ingestor_aws_role_arn           = lookup(local.ingestor_server_identity, "aws-iam-entity", "")

  # If the ingestor has a GCP service account, grant write access to
  # aws_iam_role.ingestion_bucket_writer, the IAM role configured to allow
  # assumption by the GCP SA. If the ingestor has an AWS IAM role, just grant
  # permission to it.
  ingestion_bucket_writer_role = (var.use_aws && local.ingestor_aws_role_arn == "") ? (
    aws_iam_role.ingestion_bucket_writer[0].arn
    ) : (
    local.ingestor_aws_role_arn
  )

  resource_prefix = "prio-${var.environment}-${var.data_share_processor_name}"

  canned_peer_share_processor_server_identities = {
    "ta-ta-apple" = {
      "aws-account-id"            = "994058661178"
      "gcp-service-account-email" = "mitre-collaboration-account@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "ta-ta-g-enpa" = {
      "aws-account-id"            = "994058661178"
      "gcp-service-account-email" = "mitre-collaboration-account@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ak-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ak-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ca-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ca-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-co-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-co-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ct-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ct-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-dc-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-dc-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-hi-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-hi-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-la-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-la-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ma-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ma-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-md-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-md-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-mo-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-mo-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-nm-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-nm-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-nv-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-nv-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ut-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-ut-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-va-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-va-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-wa-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-wa-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-wi-apple" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
    "us-wi-g-enpa" = {
      "aws-account-id"            = "004406395600"
      "gcp-service-account-email" = "mitre-collaboration-prod-accou@nih-nci-ext-collab-config-2d25.iam.gserviceaccount.com"
    }
  }
  peer_share_processor_server_identity               = local.canned_peer_share_processor_server_identities[var.data_share_processor_name]
  peer_share_processor_gcp_service_account_email     = local.peer_share_processor_server_identity.gcp-service-account-email
  peer_share_processor_gcp_service_account_unique_id = lookup(local.peer_share_processor_server_identity, "gcp-service-account-id", "")

  # If running EKS, we impersonate a GCP SA shared across the whole env to write
  # to the peer's validation buckets in S3.
  # If running in impure GCP, we assume the bucket writer IAM role we created.
  # If running in pure GCP, we will discover a bucket writer IAM role created by
  # the peer at runtime, in their specific manifest, but must impersonate a GCP
  # SA in order to obtain identity tokens.
  remote_validation_bucket_writer = var.use_aws ? {
    identity                                      = var.remote_bucket_writer_gcp_service_account_email
    gcp_sa_to_impersonate_while_assuming_identity = ""
    } : var.pure_gcp ? {
    identity                                      = ""
    gcp_sa_to_impersonate_while_assuming_identity = var.remote_bucket_writer_gcp_service_account_email
    } : {
    identity                                      = aws_iam_role.bucket_role[0].arn
    gcp_sa_to_impersonate_while_assuming_identity = ""
  }

  ingestion_bucket_name = "${local.resource_prefix}-ingestion"
  ingestion_bucket_url = var.use_aws ? (
    "s3://${var.aws_region}/${local.ingestion_bucket_name}"
    ) : (
    "gs://${local.ingestion_bucket_name}"
  )
  peer_validation_bucket_name = "${local.resource_prefix}-peer-validation"
  peer_validation_bucket_url = var.use_aws ? (
    "s3://${var.aws_region}/${local.peer_validation_bucket_name}"
    ) : (
    "gs://${local.peer_validation_bucket_name}"
  )
  own_validation_bucket_name = "${local.resource_prefix}-own-validation"
  own_validation_bucket_url = var.use_aws ? (
    "s3://${var.aws_region}/${local.own_validation_bucket_name}"
    ) : (
    "gs://${local.own_validation_bucket_name}"
  )

  intake_queue    = var.use_aws ? module.sns_sqs["intake"].queue : module.pubsub["intake"].queue
  aggregate_queue = var.use_aws ? module.sns_sqs["aggregate"].queue : module.pubsub["aggregate"].queue
}

# For data share processors that run in GKE but are not pure GKE, or ones that
# run in EKS, we create an IAM role to enable writing to the peer's validation
# buckets in S3.
# https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_oidc.html
resource "aws_iam_role" "bucket_role" {
  count = var.use_aws || !var.pure_gcp ? 1 : 0

  name = "${local.resource_prefix}-bucket-role"
  # Since azp is set in the auth token GCP generates, we must check oaud in the
  # role assumption policy, and the value must match what we request when
  # requesting identity tokens from the GCP API
  # https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_iam-condition-keys.html
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          Federated = "accounts.google.com"
        }
        Action = "sts:AssumeRoleWithWebIdentity"
        Condition = {
          StringEquals = {
            # When running in EKS, we allow the peer's GCP service account to
            # assume this role to write to our buckets.
            # When running in non-pure GCP, we allow our data share processor's
            # GCP service account to assume the role to write to the peer's
            # validation buckets.
            # When running in pure GCP, we don't create this role at all.
            "accounts.google.com:sub"  = var.use_aws ? local.peer_share_processor_gcp_service_account_unique_id : module.kubernetes.gcp_service_account_unique_id
            "accounts.google.com:oaud" = "sts.amazonaws.com/gke-identity-federation"
          }
        }
      }
    ]
  })
  permissions_boundary = var.role_permissions_boundary_policy_arn
}

# The bucket_role defined above is created in our AWS account and used to
# access S3 buckets in a peer's AWS account. Besides _their_ bucket having a
# policy that grants us access, we need to make sure _our_ role allows use of S3
# API, hence this policy. Note it is not the same as the role assumption policy
# defined inline in aws_iam_role.bucket_role.
# TODO: Make this policy as restrictive as possible:
# https://github.com/abetterinternet/prio-server/issues/140
resource "aws_iam_role_policy" "bucket_role_policy" {
  count = var.use_aws || !var.pure_gcp ? 1 : 0

  name = "${local.resource_prefix}-bucket-role-policy"
  role = aws_iam_role.bucket_role[0].id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action   = ["s3:*"]
        Effect   = "Allow"
        Resource = "*"
      }
    ]
  })
}

# If our ingestion bucket is in S3 and this data share processor's ingestion
# server does not have an AWS IAM identity, then we must create an AWS IAM role
# for it to assume using its GCP service account.
resource "aws_iam_role" "ingestion_bucket_writer" {
  count = (var.use_aws && local.ingestor_aws_role_arn == "") ? 1 : 0

  name = "${local.resource_prefix}-ingestion-bucket-writer"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          Federated = "accounts.google.com"
        }
        Action = "sts:AssumeRoleWithWebIdentity"
        Condition = {
          StringEquals = {
            "accounts.google.com:sub" = local.ingestor_gcp_service_account_id
          }
        }
      }
    ]
  })
}

module "cloud_storage_aws" {
  count  = var.use_aws ? 1 : 0
  source = "../../modules/cloud_storage_aws"

  resource_prefix               = local.resource_prefix
  bucket_reader                 = module.kubernetes.aws_iam_role.arn
  ingestion_bucket_name         = local.ingestion_bucket_name
  ingestion_bucket_writer       = local.ingestion_bucket_writer_role
  peer_validation_bucket_name   = local.peer_validation_bucket_name
  peer_validation_bucket_writer = aws_iam_role.bucket_role[0].arn
  own_validation_bucket_name    = "${local.resource_prefix}-own-validation"
  own_validation_bucket_writer  = module.kubernetes.aws_iam_role.arn
}

module "cloud_storage_gcp" {
  count  = var.use_aws ? 0 : 1
  source = "../../modules/cloud_storage_gcp"

  resource_prefix               = local.resource_prefix
  gcp_region                    = var.gcp_region
  kms_keyring                   = var.kms_keyring
  bucket_reader                 = module.kubernetes.gcp_service_account_email
  ingestion_bucket_name         = local.ingestion_bucket_name
  ingestion_bucket_writer       = local.ingestor_server_identity.gcp-service-account-email
  peer_validation_bucket_name   = local.peer_validation_bucket_name
  peer_validation_bucket_writer = local.peer_share_processor_gcp_service_account_email
  own_validation_bucket_name    = "${local.resource_prefix}-own-validation"
  own_validation_bucket_writer  = module.kubernetes.gcp_service_account_email
}

module "pubsub" {
  for_each                   = toset(var.use_aws ? [] : ["intake", "aggregate"])
  source                     = "../../modules/pubsub"
  environment                = var.environment
  data_share_processor_name  = var.data_share_processor_name
  publisher_service_account  = module.kubernetes.gcp_service_account_email
  subscriber_service_account = module.kubernetes.gcp_service_account_email
  task                       = each.key
}

module "sns_sqs" {
  for_each                  = toset(var.use_aws ? ["intake", "aggregate"] : [])
  source                    = "../../modules/sns_sqs"
  environment               = var.environment
  data_share_processor_name = var.data_share_processor_name
  publisher_iam_role        = module.kubernetes.aws_iam_role.arn
  subscriber_iam_role       = module.kubernetes.aws_iam_role.arn
  task                      = each.key
}

module "kubernetes" {
  source                                    = "../../modules/kubernetes/"
  data_share_processor_name                 = var.data_share_processor_name
  ingestor                                  = var.ingestor
  environment                               = var.environment
  kubernetes_namespace                      = var.kubernetes_namespace
  ingestion_bucket                          = local.ingestion_bucket_url
  ingestor_manifest_base_url                = var.ingestor_manifest_base_url
  packet_decryption_key_kubernetes_secret   = var.packet_decryption_key_kubernetes_secret
  peer_manifest_base_url                    = var.peer_share_processor_manifest_base_url
  remote_peer_validation_bucket_identity    = local.remote_validation_bucket_writer
  peer_validation_bucket                    = local.peer_validation_bucket_url
  own_validation_bucket                     = local.own_validation_bucket_url
  sum_part_bucket_service_account_email     = var.remote_bucket_writer_gcp_service_account_email
  portal_server_manifest_base_url           = var.portal_server_manifest_base_url
  is_first                                  = var.is_first
  intake_max_age                            = var.intake_max_age
  aggregation_period                        = var.aggregation_period
  aggregation_grace_period                  = var.aggregation_grace_period
  pushgateway                               = var.pushgateway
  container_registry                        = var.container_registry
  workflow_manager_image                    = var.workflow_manager_image
  workflow_manager_version                  = var.workflow_manager_version
  facilitator_image                         = var.facilitator_image
  facilitator_version                       = var.facilitator_version
  intake_queue                              = local.intake_queue
  aggregate_queue                           = local.aggregate_queue
  min_intake_worker_count                   = var.min_intake_worker_count
  max_intake_worker_count                   = var.max_intake_worker_count
  min_aggregate_worker_count                = var.min_aggregate_worker_count
  max_aggregate_worker_count                = var.max_aggregate_worker_count
  use_aws                                   = var.use_aws
  eks_oidc_provider                         = var.eks_oidc_provider
  aws_region                                = var.aws_region
  gcp_workload_identity_pool_provider       = var.gcp_workload_identity_pool_provider
  single_object_validation_batch_localities = var.single_object_validation_batch_localities
  enable_heap_profiles                      = var.enable_heap_profiles
  profile_nfs_server                        = var.profile_nfs_server
}

output "data_share_processor_name" {
  value = var.data_share_processor_name
}

output "ingestor_name" {
  value = var.ingestor
}

output "kubernetes_namespace" {
  value = var.kubernetes_namespace
}

output "certificate_fqdn" {
  value = "${var.kubernetes_namespace}.${var.certificate_domain}"
}

output "gcp_service_account_email" {
  value = module.kubernetes.gcp_service_account_email
}

output "aws_iam_role" {
  value = module.kubernetes.aws_iam_role
}

output "aggregate_queue" {
  value = local.aggregate_queue
}

output "specific_manifest" {
  value = {
    format                   = 2
    ingestion-identity       = var.use_aws ? local.ingestion_bucket_writer_role : null
    ingestion-bucket         = local.ingestion_bucket_url
    peer-validation-identity = (var.pure_gcp && var.use_aws) ? aws_iam_role.bucket_role[0].arn : null
    peer-validation-bucket   = local.peer_validation_bucket_url
    batch-signing-public-keys = {
      (module.kubernetes.batch_signing_key) = {
        public-key = ""
        expiration = ""
      }
    }
    packet-encryption-keys = {
      (var.packet_decryption_key_kubernetes_secret) = {
        certificate-signing-request = ""
      }
    }
  }
}
