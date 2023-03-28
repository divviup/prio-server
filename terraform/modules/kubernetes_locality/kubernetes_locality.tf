variable "environment" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "key_rotator_image" {
  type = string
}

variable "key_rotator_version" {
  type = string
}

variable "use_aws" {
  type = bool
}

variable "gcp_project" {
  type = string
}

variable "gcp_secret_writer_role_id" {
  type = string
}

variable "eks_oidc_provider" {
  type = object({
    arn = string
    url = string
  })
}

variable "manifest_bucket" {
  type = object({
    bucket         = string
    bucket_url     = string
    aws_bucket_arn = string
    aws_region     = string
  })
}

variable "kubernetes_namespace" {
  type = string
}

variable "locality" {
  type = string
}

variable "ingestors" {
  type = list(string)
}

variable "certificate_fqdn" {
  type = string
}

variable "pushgateway" {
  type = string
}

variable "batch_signing_key_rotation_policy" {
  type = object({
    create_min_age   = string
    primary_min_age  = string
    delete_min_age   = string
    delete_min_count = number
  })
}

variable "packet_encryption_key_rotation_policy" {
  type = object({
    create_min_age   = string
    primary_min_age  = string
    delete_min_age   = string
    delete_min_count = number
  })
}

variable "key_rotator_schedule" {
  type = string
}

variable "enable_key_rotator_localities" {
  type        = set(string)
  description = <<DESCRIPTION
A set of localities where the key rotator is allowed to run. The special value
"*" indicates that the key rotator should run in all localities.
DESCRIPTION
}

variable "specific_manifest_templates" {
  type = map(any)
}

variable "enable_heap_profiles" {
  type = bool
}

variable "profile_nfs_server" {
  type = string
}

locals {
  iam_entity_name = "${var.environment}-${var.locality}-key-rotator"

  relevant_keyless_manifest_templates = {
    for ingestor in var.ingestors :
    ingestor => {
      for k, v in var.specific_manifest_templates["${var.locality}-${ingestor}"] :
      k => v if k != "batch-signing-public-keys" && k != "packet-encryption-keys"
    }
  }
}

module "key_rotator_account" {
  source                          = "../account_mapping"
  gcp_service_account_name        = var.use_aws ? "" : local.iam_entity_name
  gcp_project                     = var.gcp_project
  aws_iam_role_name               = var.use_aws ? local.iam_entity_name : ""
  eks_oidc_provider               = var.eks_oidc_provider
  kubernetes_service_account_name = "key-rotator"
  kubernetes_namespace            = var.kubernetes_namespace
  environment                     = var.environment
}

resource "kubernetes_role" "key_rotator_role" {
  metadata {
    name      = "key-rotator-role"
    namespace = var.kubernetes_namespace
  }

  rule {
    api_groups = [""]
    resources  = ["secrets"]
    verbs = [
      "create",
      "list",
      "get",
      "delete",
      "update",
    ]
  }
}

resource "kubernetes_role_binding" "key_rotator_role_binding" {
  metadata {
    name      = "key-rotator-role-binding"
    namespace = var.kubernetes_namespace
  }

  role_ref {
    kind      = "Role"
    name      = kubernetes_role.key_rotator_role.metadata[0].name
    api_group = "rbac.authorization.k8s.io"
  }

  subject {
    kind      = "ServiceAccount"
    name      = module.key_rotator_account.kubernetes_service_account_name
    namespace = var.kubernetes_namespace
  }
}

resource "aws_iam_role_policy" "key_rotator_manifest_bucket_writer" {
  count = var.use_aws ? 1 : 0

  name = "prio-${var.environment}-${var.locality}-key-rotator-manifest-bucket-writer"
  role = module.key_rotator_account.aws_iam_role.name
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "PutObject"
        Effect = "Allow"
        Action = [
          "s3:PutObject",
          "s3:PutObjectAcl",
          "s3:GetObject",
          "s3:GetObjectAcl",
        ]
        Resource = "${var.manifest_bucket.aws_bucket_arn}/*",
      }
    ]
  })
}

resource "aws_iam_role_policy" "key_rotator_secret_writer" {
  count = var.use_aws ? 1 : 0

  name = "prio-${var.environment}-${var.locality}-key-rotator-secret-writer"
  role = module.key_rotator_account.aws_iam_role.name
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "WriteSecret"
        Effect = "Allow"
        Action = [
          "secretsmanager:CreateSecret",
          "secretsmanager:PutSecretValue",
        ]
        Resource = "*",
      }
    ]
  })
}

resource "google_storage_bucket_iam_member" "key_rotator_manifest_bucket_writer" {
  count = var.use_aws ? 0 : 1

  bucket = var.manifest_bucket.bucket
  role   = "roles/storage.legacyBucketWriter"
  member = "serviceAccount:${module.key_rotator_account.gcp_service_account_email}"
}

resource "google_project_iam_member" "key_rotator_in_secret_writer" {
  count = var.use_aws ? 0 : 1

  project = var.gcp_project
  role    = var.gcp_secret_writer_role_id
  member  = "serviceAccount:${module.key_rotator_account.gcp_service_account_email}"
}

resource "kubernetes_cron_job" "key_rotator" {
  metadata {
    name      = "${var.environment}-key-rotator-${var.locality}"
    namespace = var.kubernetes_namespace
  }

  spec {
    schedule                      = var.key_rotator_schedule
    concurrency_policy            = "Forbid"
    successful_jobs_history_limit = 5
    failed_jobs_history_limit     = 5

    job_template {
      metadata {}
      spec {
        template {
          metadata {}
          spec {
            # Allow key-rotator to run on both spot and non-spot nodes, with
            # no preference.
            toleration {
              key      = "divviup.org/spot-vm"
              operator = "Exists"
              effect   = "NoSchedule"
            }

            container {
              name  = "key-rotator"
              image = "${var.container_registry}/${var.key_rotator_image}:${var.key_rotator_version}"
              resources {
                requests = {
                  memory = "128Mi"
                  cpu    = "0.5"
                }
                limits = {
                  memory = "512Mi"
                  cpu    = "1"
                }
              }

              args = [
                "--prio-environment=${var.environment}",
                "--kubernetes-namespace=${var.kubernetes_namespace}",
                "--manifest-bucket-url=${var.manifest_bucket.bucket_url}",
                "--locality=${var.locality}",
                "--ingestors=${join(",", var.ingestors)}",
                "--csr-fqdn=${var.certificate_fqdn}",
                "--aws-region=${var.manifest_bucket.aws_region}",
                "--push-gateway=${var.pushgateway}",
                "--backup=${var.use_aws ? "aws" : "gcp:${var.gcp_project}"}",
                "--default-manifest-by-ingestor=${jsonencode(local.relevant_keyless_manifest_templates)}",
                "--dry-run=${!(contains(var.enable_key_rotator_localities, "*") || contains(var.enable_key_rotator_localities, var.locality))}",

                "--batch-signing-key-enable-rotation=false",
                "--batch-signing-key-create-min-age=${var.batch_signing_key_rotation_policy.create_min_age}",
                "--batch-signing-key-primary-min-age=${var.batch_signing_key_rotation_policy.primary_min_age}",
                "--batch-signing-key-delete-min-age=${var.batch_signing_key_rotation_policy.delete_min_age}",
                "--batch-signing-key-delete-min-count=${var.batch_signing_key_rotation_policy.delete_min_count}",

                "--packet-encryption-key-enable-rotation=true",
                "--packet-encryption-key-create-min-age=${var.packet_encryption_key_rotation_policy.create_min_age}",
                "--packet-encryption-key-primary-min-age=${var.packet_encryption_key_rotation_policy.primary_min_age}",
                "--packet-encryption-key-delete-min-age=${var.packet_encryption_key_rotation_policy.delete_min_age}",
                "--packet-encryption-key-delete-min-count=${var.packet_encryption_key_rotation_policy.delete_min_count}",
                "--memprofile", var.enable_heap_profiles ? "/profiles/mem-$(NAMESPACE)-$(POD).pb.gz" : "",
              ]
              env {
                name = "NAMESPACE"
                value_from {
                  field_ref {
                    field_path = "metadata.namespace"
                  }
                }
              }
              env {
                name = "POD"
                value_from {
                  field_ref {
                    field_path = "metadata.name"
                  }
                }
              }
              dynamic "volume_mount" {
                for_each = var.profile_nfs_server != null ? [0] : []
                content {
                  mount_path = "/profiles"
                  name       = "profiles"
                }
              }
            }

            # If we use any other restart policy, then when the job is finally
            # deemed to be a failure, Kubernetes will destroy the job, pod and
            # container(s) virtually immediately. This can cause us to lose logs
            # if the container is reaped before the GKE logging agent can upload
            # logs. Since this is a cronjob and we will retry anyway, we use
            # "Never".
            # https://kubernetes.io/docs/concepts/workloads/controllers/job/#handling-pod-and-container-failures
            # https://github.com/kubernetes/kubernetes/issues/74848
            restart_policy                  = "Never"
            service_account_name            = module.key_rotator_account.kubernetes_service_account_name
            automount_service_account_token = true
            dynamic "volume" {
              for_each = var.profile_nfs_server != null ? [0] : []
              content {
                name = "profiles"
                nfs {
                  server = var.profile_nfs_server
                  path   = "/"
                }
              }
            }
          }
        }
      }
    }
  }
}
