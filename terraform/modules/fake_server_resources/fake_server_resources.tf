variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "manifest_bucket" {
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
    intake_worker_count : string
    aggregate_worker_count : string
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
          "accounts.google.com:oaud": "sts.amazonaws.com/${data.aws_caller_identity.current.account_id}"
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
  source      = "../account_mapping"
  environment = var.environment

  gcp_service_account_name        = "${var.environment}-fake-ingestion-identity"
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
    # We want this to be a Terraform resource that can be managed and destroyed
    # by this module, but we do not want the cleartext private key to appear in
    # the TF statefile. So we set a dummy value here, and will update the value
    # later using kubectl. We use lifecycle.ignore_changes so that Terraform
    # won't blow away the replaced value on subsequent applies.
    secret_key = "not-a-real-key"
  }

  lifecycle {
    ignore_changes = [
      data["secret_key"]
    ]
  }
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
          env {
            name  = "AWS_ACCOUNT_ID"
            value = data.aws_caller_identity.current.account_id
          }
          env {
            name  = "RUST_BACKTRACE"
            value = "FULL"
          }
          env {
            name  = "RUST_LOG"
            value = "info"
          }
          args = [
            "--pushgateway", var.pushgateway,
            "generate-ingestion-sample-worker",
            "--ingestor-name", each.value.ingestor,
            "--locality-name", each.value.locality,
            "--kube-namespace", kubernetes_namespace.tester.metadata[0].name,
            "--ingestor-manifest-base-url", "https://${each.value.ingestor_manifest_base_url}",
            "--pha-manifest-base-url", "https://${each.value.peer_share_processor_manifest_base_url}",
            "--facilitator-manifest-base-url", "https://${var.own_manifest_base_url}",
            "--aggregation-id", "kittens-seen",
            "--packet-count", "10",
            "--batch-start-time", "1000000000",
            "--batch-end-time", "1000000100",
            "--dimension", "123",
            "--epsilon", "0.23",
            "--generation-interval", "60"
          ]
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
