variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "own_manifest_base_url" {
  type = string
}

variable "ingestor_pairs" {
  type = map(object({
    locality : string
    ingestor : string
    kubernetes_namespace : string
    packet_decryption_key_kubernetes_secret : string
    ingestor_manifest_base_url : string
    peer_share_processor_manifest_base_url : string
  }))
}

variable "pushgateway" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "facilitator_image" {
  type = string
}

variable "facilitator_version" {
  type = string
}

variable "manifest_bucket" {
  type = string
}

resource "kubernetes_namespace" "tester" {
  metadata {
    name = "tester"
    annotations = {
      environment = var.environment
    }
  }
}

data "aws_caller_identity" "current" {}

resource "aws_iam_role" "tester_role" {
  name = "prio-${var.environment}-integration-tester"
  # Since azp is set in the auth token Google generates, we must check oaud in
  # the role assumption policy, and the value must match what we request when
  # requesting tokens from the GKE metadata service in
  # S3Transport::new_with_client
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
          "accounts.google.com:sub": "${module.account_mapping.gcp_service_account_unique_id}",
          "accounts.google.com:oaud": "sts.amazonaws.com/gke-identity-federation"
        }
      }
    }
  ]
}
ROLE
}

resource "aws_iam_role_policy" "bucket_role_policy" {
  name = "prio-${var.environment}-integration-tester-policy"
  role = aws_iam_role.tester_role.id

  policy = <<POLICY
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Action": [
        "s3:*"
      ],
      "Effect": "Allow",
      "Resource": "*"
    }
  ]
}
POLICY
}

module "account_mapping" {
  source                          = "../account_mapping"
  environment                     = var.environment
  gcp_service_account_name        = "${var.environment}-fake-ingestion-identity"
  gcp_project                     = var.gcp_project
  kubernetes_service_account_name = "ingestion-identity"
  kubernetes_namespace            = kubernetes_namespace.tester.metadata[0].name
}

resource "kubernetes_role" "integration_tester_role" {
  metadata {
    name      = "${var.environment}-ingestion-tester-role"
    namespace = kubernetes_namespace.tester.metadata[0].name
  }

  rule {
    api_groups = [""]
    // integration-tester needs to be able to list and get secrets.
    // This is used by the integration-tester to find a valid
    // batch signing key for itself.
    resources = ["secrets"]
    verbs     = ["list", "get"]
  }
}


resource "kubernetes_role_binding" "integration_tester_role_binding" {
  metadata {
    name      = "${var.environment}-integration-tester-can-admin"
    namespace = kubernetes_namespace.tester.metadata[0].name
  }

  role_ref {
    kind      = "Role"
    name      = kubernetes_role.integration_tester_role.metadata[0].name
    api_group = "rbac.authorization.k8s.io"
  }

  subject {
    kind      = "ServiceAccount"
    name      = module.account_mapping.kubernetes_service_account_name
    namespace = kubernetes_namespace.tester.metadata[0].name
  }
}

resource "kubernetes_secret" "batch_signing_key" {
  metadata {
    generate_name = "batch-signing-key"
    namespace     = kubernetes_namespace.tester.metadata[0].name
    labels = {
      "isrg-prio.org/type" : "batch-signing-key"
    }
  }

  data = {
    # This is the base64 encoding of a PKCS#8 document containing a P-256
    # private key obtained from a development environment. Hard-coding this key
    # here absolves us of generating it elsewhere, and since it is only used in
    # test deployments, there's no hard in leaking it in git or in Terraform
    # state.
    secret_key = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgRGhfadpOZSZuMdBrjO7SqvJsJSTqTFgfDPh9bTr4MBihRANCAATq6yKefdZ2vOnp5xed74OWuHhlGW27wOvMe8cUVJ5XbS3orlg1PVmH+lHFZy7VtcGxV8WU1YAxDayZbGZ2/Te3"
  }
}

resource "google_storage_bucket_object" "global_manifest" {
  name          = "singleton-ingestor/global-manifest.json"
  bucket        = var.manifest_bucket
  content_type  = "application/json"
  cache_control = "no-cache"
  content = jsonencode({
    format = 1
    server-identity = {
      aws-iam-entity            = aws_iam_role.tester_role.arn
      gcp-service-account-id    = module.account_mapping.gcp_service_account_unique_id
      gcp-service-account-email = module.account_mapping.gcp_service_account_email
    }
    batch-signing-public-keys = {
      (kubernetes_secret.batch_signing_key.metadata[0].name) = {
        # This public key corresponds to the private key in kubernetes_secret.batch_signing_key
        public-key = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE6usinn3Wdrzp6ecXne+Dlrh4ZRlt\nu8DrzHvHFFSeV20t6K5YNT1Zh/pRxWcu1bXBsVfFlNWAMQ2smWxmdv03tw==\n-----END PUBLIC KEY-----\n"
        expiration = "2099-10-05T23:18:59Z"
      }
    }
  })
}

resource "kubernetes_deployment" "integration-tester" {
  for_each = var.ingestor_pairs

  metadata {
    name      = "integration-tester-${each.value.locality}-${each.value.ingestor}"
    namespace = kubernetes_namespace.tester.metadata[0].name
  }

  spec {
    replicas = 1
    selector {
      match_labels = {
        app      = "integration-tester-worker"
        ingestor = each.value.ingestor
      }
    }
    template {
      metadata {
        labels = {
          app      = "integration-tester-worker"
          ingestor = each.value.ingestor
        }
      }
      spec {
        service_account_name            = module.account_mapping.kubernetes_service_account_name
        automount_service_account_token = true
        container {
          name  = "integration-tester"
          image = "${var.container_registry}/${var.facilitator_image}:${var.facilitator_version}"
          args = [
            "generate-ingestion-sample-worker",
            "--ingestor-name", each.value.ingestor,
            "--locality-name", each.value.locality,
            "--pha-manifest-base-url", "https://${each.value.peer_share_processor_manifest_base_url}",
            "--facilitator-manifest-base-url", "https://${var.own_manifest_base_url}",
            "--batch-signing-private-key-default-identifier=${kubernetes_secret.batch_signing_key.metadata[0].name}",
            "--aggregation-id", "kittens-seen",
            "--packet-count", "10",
            "--batch-start-time", "1000000000",
            "--batch-end-time", "1000000100",
            "--dimension", "123",
            "--epsilon", "0.23",
            "--generation-interval", "60",
            "--worker-maximum-lifetime", "3600",
          ]
          env {
            name = "BATCH_SIGNING_PRIVATE_KEY"
            value_from {
              secret_key_ref {
                name     = kubernetes_secret.batch_signing_key.metadata[0].name
                key      = "secret_key"
                optional = false
              }
            }
          }
          env {
            name  = "RUST_BACKTRACE"
            value = "FULL"
          }
          env {
            name  = "RUST_LOG"
            value = "info"
          }
        }
      }
    }
  }
}

output "aws_iam_entity" {
  value = aws_iam_role.tester_role.arn
}

output "gcp_service_account_id" {
  value = module.account_mapping.gcp_service_account_unique_id
}

output "gcp_service_account_email" {
  value = module.account_mapping.gcp_service_account_email
}

output "test_kubernetes_namespace" {
  value = kubernetes_namespace.tester.metadata[0].name
}

output "batch_signing_key_name" {
  value = kubernetes_secret.batch_signing_key.metadata[0].name
}
