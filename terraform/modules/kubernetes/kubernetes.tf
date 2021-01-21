variable "ingestor" {
  type = string
}

variable "data_share_processor_name" {
  type = string
}

variable "environment" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "workflow_manager_image" {
  type    = string
  default = "prio-workflow-manager"
}

variable "workflow_manager_version" {
  type = string
}

variable "facilitator_image" {
  type    = string
  default = "prio-facilitator"
}

variable "facilitator_version" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "ingestion_bucket" {
  type = string
}

variable "ingestion_bucket_identity" {
  type = string
}

variable "ingestor_manifest_base_url" {
  type = string
}

variable "peer_validation_bucket" {
  type = string
}

variable "peer_validation_bucket_identity" {
  type = string
}

variable "peer_manifest_base_url" {
  type = string
}

variable "remote_peer_validation_bucket_identity" {
  type = string
}

variable "own_validation_bucket" {
  type = string
}

variable "own_manifest_base_url" {
  type = string
}

variable "kubernetes_namespace" {
  type = string
}

variable "packet_decryption_key_kubernetes_secret" {
  type = string
}

variable "portal_server_manifest_base_url" {
  type = string
}

variable "sum_part_bucket_service_account_email" {
  type = string
}

variable "is_env_with_ingestor" {
  type = bool
}

variable "test_peer_ingestion_bucket" {
  type = string
}

variable "is_first" {
  type = bool
}

variable "aggregation_period" {
  type = string
}

variable "aggregation_grace_period" {
  type = string
}

variable "pushgateway" {
  type = string
}

variable "intake_queue" {
  type = string
}

variable "aggregate_queue" {
  type = string
}

variable "intake_worker_count" {
  type = number
}

variable "aggregate_worker_count" {
  type = number
}

data "aws_caller_identity" "current" {}

# Workload identity[1] lets us map GCP service accounts to Kubernetes service
# accounts. We need this so that pods can use GCP API, but also AWS APIs like S3
# via Web Identity Federation. To use the credentials, the container must fetch
# the authentication token from the instance metadata service. Kubernetes has
# features for automatically providing a service account token (e.g. via a
# a mounted volume[2]), but that would be a token for the *Kubernetes* level
# service account, and not the one we can present to AWS.
# [1] https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity
# [2] https://kubernetes.io/docs/tasks/configure-pod-container/configure-service-account/#service-account-token-volume-projection

module "account_mapping" {
  source                  = "../account_mapping"
  google_account_name     = "${var.environment}-${var.data_share_processor_name}-workflow-manager"
  kubernetes_account_name = "${var.data_share_processor_name}-workflow-manager"
  kubernetes_namespace    = var.kubernetes_namespace
  environment             = var.environment
  gcp_project             = var.gcp_project
}

# Allows the Kubernetes service account to request auth tokens for the GCP
# service account.
resource "google_service_account_iam_binding" "workflow_manager_token" {
  service_account_id = module.account_mapping.google_service_account_name
  role               = "roles/iam.serviceAccountTokenCreator"
  members = [
    module.account_mapping.service_account
  ]
}

resource "kubernetes_secret" "batch_signing_key" {
  metadata {
    name      = "${var.environment}-${var.data_share_processor_name}-batch-signing-key"
    namespace = var.kubernetes_namespace
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

# ConfigMap containing the parameters that are common to every intake-batch task
# that will be handled in this data share processor, except for secrets.
resource "kubernetes_config_map" "intake_batch_config_map" {
  metadata {
    name      = "${var.data_share_processor_name}-intake-batch-config"
    namespace = var.kubernetes_namespace
  }

  # These key-value pairs will be plopped directly into the container
  # environment, so they MUST match the environment variables set in various
  # Arg::env calls in src/bin/facilitator.rs.
  data = {
    # PACKET_DECRYPTION_KEYS is a Kubernetes secret
    # BATCH_SIGNING_PRIVATE_KEY is a Kubernetes secret
    IS_FIRST                             = var.is_first ? "true" : "false"
    AWS_ACCOUNT_ID                       = data.aws_caller_identity.current.account_id
    BATCH_SIGNING_PRIVATE_KEY_IDENTIFIER = kubernetes_secret.batch_signing_key.metadata[0].name
    INGESTOR_IDENTITY                    = var.ingestion_bucket_identity
    INGESTOR_INPUT                       = var.ingestion_bucket
    INGESTOR_MANIFEST_BASE_URL           = "https://${var.ingestor_manifest_base_url}"
    INSTANCE_NAME                        = var.data_share_processor_name
    PEER_IDENTITY                        = var.remote_peer_validation_bucket_identity
    PEER_MANIFEST_BASE_URL               = "https://${var.peer_manifest_base_url}"
    OWN_OUTPUT                           = var.own_validation_bucket
    RUST_LOG                             = "info"
    RUST_BACKTRACE                       = "1"
    PUSHGATEWAY                          = var.pushgateway
    TASK_QUEUE_KIND                      = "gcp-pubsub"
    TASK_QUEUE_NAME                      = var.intake_queue
    GCP_PROJECT_ID                       = data.google_project.project.project_id

  }
}

# ConfigMap containing the parameters that are common to every aggregation task
# that will be handled in this data share processor, except for secrets.
resource "kubernetes_config_map" "aggregate_config_map" {
  metadata {
    name      = "${var.data_share_processor_name}-aggregate-config"
    namespace = var.kubernetes_namespace
  }

  data = {
    # PACKET_DECRYPTION_KEYS is a Kubernetes secret
    # BATCH_SIGNING_PRIVATE_KEY is a Kubernetes secret
    IS_FIRST                             = var.is_first ? "true" : "false"
    AWS_ACCOUNT_ID                       = data.aws_caller_identity.current.account_id
    BATCH_SIGNING_PRIVATE_KEY_IDENTIFIER = kubernetes_secret.batch_signing_key.metadata[0].name
    INGESTOR_INPUT                       = var.ingestion_bucket
    INGESTOR_IDENTITY                    = var.ingestion_bucket_identity
    INGESTOR_MANIFEST_BASE_URL           = "https://${var.ingestor_manifest_base_url}"
    INSTANCE_NAME                        = var.data_share_processor_name
    OWN_INPUT                            = var.own_validation_bucket
    OWN_MANIFEST_BASE_URL                = "https://${var.own_manifest_base_url}"
    PEER_INPUT                           = var.peer_validation_bucket
    PEER_IDENTITY                        = var.peer_validation_bucket_identity
    PEER_MANIFEST_BASE_URL               = "https://${var.peer_manifest_base_url}"
    PORTAL_IDENTITY                      = var.sum_part_bucket_service_account_email
    PORTAL_MANIFEST_BASE_URL             = "https://${var.portal_server_manifest_base_url}"
    RUST_LOG                             = "info"
    RUST_BACKTRACE                       = "1"
    PUSHGATEWAY                          = var.pushgateway
    TASK_QUEUE_KIND                      = "gcp-pubsub"
    TASK_QUEUE_NAME                      = var.aggregate_queue
    GCP_PROJECT_ID                       = data.google_project.project.project_id
  }
}

data "google_project" "project" {}

resource "kubernetes_cron_job" "workflow_manager" {
  metadata {
    name      = "workflow-manager-${var.ingestor}-${var.environment}"
    namespace = var.kubernetes_namespace

    annotations = {
      environment = var.environment
    }
  }
  spec {
    schedule                      = "*/10 * * * *"
    concurrency_policy            = "Forbid"
    successful_jobs_history_limit = 5
    failed_jobs_history_limit     = 3
    job_template {
      metadata {}
      spec {
        template {
          metadata {}
          spec {
            container {
              name  = "workflow-manager"
              image = "${var.container_registry}/${var.workflow_manager_image}:${var.workflow_manager_version}"
              resources {
                requests {
                  memory = "500Mi"
                  cpu    = "0.5"
                }
                limits {
                  memory = "2Gi"
                  cpu    = "1.5"
                }
              }

              args = [
                "--aggregation-period", var.aggregation_period,
                "--grace-period", var.aggregation_grace_period,
                "--intake-max-age", "24h",
                "--is-first=${var.is_first ? "true" : "false"}",
                "--k8s-namespace", var.kubernetes_namespace,
                "--ingestor-label", var.ingestor,
                "--ingestor-input", var.ingestion_bucket,
                "--ingestor-identity", var.ingestion_bucket_identity,
                "--own-validation-input", var.own_validation_bucket,
                "--peer-validation-input", var.peer_validation_bucket,
                "--peer-validation-identity", var.peer_validation_bucket_identity,
                "--push-gateway", var.pushgateway,
                "--task-queue-kind", "gcp-pubsub",
                "--intake-tasks-topic", var.intake_queue,
                "--aggregate-tasks-topic", var.aggregate_queue,
                "--gcp-project-id", data.google_project.project.project_id,
              ]
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
            service_account_name            = module.account_mapping.kubernetes_account_name
            automount_service_account_token = true
          }
        }
      }
    }
  }
}

resource "kubernetes_deployment" "intake_batch" {
  metadata {
    name      = "intake-batch-${var.ingestor}"
    namespace = var.kubernetes_namespace
  }

  spec {
    replicas = var.intake_worker_count
    selector {
      match_labels = {
        name = "intake-batch-${var.ingestor}"
      }
    }
    template {
      metadata {
        labels = {
          name = "intake-batch-${var.ingestor}"
        }
      }
      spec {
        service_account_name = module.account_mapping.kubernetes_account_name
        container {
          name  = "facile-container"
          image = "${var.container_registry}/${var.facilitator_image}:${var.facilitator_version}"
          args  = ["intake-batch-worker"]
          resources {
            # Batch intake is single threaded, and we never expect to see
            # batches larger than 3-400 MB, so set the limits such that we can
            # process whole batches in memory (plus a safety margin) and we can
            # use an entire core if necessary. However we set the requests much
            # lower since most batches are much smaller than 3-400 MB and we get
            # more efficient bin packing this way.
            requests {
              memory = "50Mi"
              cpu    = "0.1"
            }
            limits {
              memory = "550Mi"
              cpu    = "1.5"
            }
          }
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
            name = "PACKET_DECRYPTION_KEYS"
            value_from {
              secret_key_ref {
                name     = var.packet_decryption_key_kubernetes_secret
                key      = "secret_key"
                optional = false
              }
            }
          }
          env_from {
            config_map_ref {
              name     = kubernetes_config_map.intake_batch_config_map.metadata[0].name
              optional = false
            }
          }
        }
      }
    }
  }
}

resource "kubernetes_deployment" "aggregate" {
  metadata {
    name      = "aggregate-${var.ingestor}"
    namespace = var.kubernetes_namespace
  }

  spec {
    replicas = var.aggregate_worker_count
    selector {
      match_labels = {
        name = "aggregate-${var.ingestor}"
      }
    }
    template {
      metadata {
        labels = {
          name = "aggregate-${var.ingestor}"
        }
      }
      spec {
        service_account_name = module.account_mapping.kubernetes_account_name
        container {
          name  = "facile-container"
          image = "${var.container_registry}/${var.facilitator_image}:${var.facilitator_version}"
          args  = ["aggregate-worker"]
          resources {
            # As in the intake-batch case, aggregate jobs are single threaded
            # and need to fit whole ingestion batches into memory.
            requests {
              memory = "50Mi"
              cpu    = "0.1"
            }
            limits {
              memory = "550Mi"
              cpu    = "1.5"
            }
          }
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
            name = "PACKET_DECRYPTION_KEYS"
            value_from {
              secret_key_ref {
                name     = var.packet_decryption_key_kubernetes_secret
                key      = "secret_key"
                optional = false
              }
            }
          }
          env_from {
            config_map_ref {
              name     = kubernetes_config_map.aggregate_config_map.metadata[0].name
              optional = false
            }
          }
        }
      }
    }
  }
}

resource "kubernetes_cron_job" "sample_maker" {
  # This sample maker acts as an ingestion server in our test setup. It only
  # gets created in one of the two envs, and writes to both env's ingestion
  # buckets.
  count = var.is_env_with_ingestor ? 1 : 0
  metadata {
    name      = "sample-maker-${var.ingestor}-${var.environment}"
    namespace = var.kubernetes_namespace

    annotations = {
      environment = var.environment
    }
  }
  spec {
    schedule                      = "* * * * *"
    concurrency_policy            = "Forbid"
    successful_jobs_history_limit = 5
    failed_jobs_history_limit     = 3
    job_template {
      metadata {}
      spec {
        template {
          metadata {}
          spec {
            restart_policy                  = "Never"
            service_account_name            = module.account_mapping.kubernetes_account_name
            automount_service_account_token = true
            container {
              name  = "sample-maker"
              image = "${var.container_registry}/${var.facilitator_image}:${var.facilitator_version}"
              args = [
                "--pushgateway", var.pushgateway,
                "generate-ingestion-sample",
                "--peer-output", var.ingestion_bucket,
                "--own-output", var.test_peer_ingestion_bucket,
                "--own-identity", var.remote_peer_validation_bucket_identity,
                "--aggregation-id", "kittens-seen",
                # All instances of the sample maker use the same batch signing
                # key, thus simulating being a single server.
                "--batch-signing-private-key", "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggoa08rQR90Asvhy5bWIgFBDeGaO8FnVEF3PVpNVmDGChRANCAAQ2mZfm4UC73PkWsYz3Uub6UTIAFQCPGxouP1O1PlmntOpfLYdvyZDCuenAzv1oCfyToolNArNjwo/+harNn1fs",
                "--batch-signing-private-key-identifier", "sample-maker-signing-key",
                "--packet-count", "10",
                # We use a fixed packet encryption key so that we can make sure
                # to use the same one in the corresponding data share processor
                # in the other env.
                "--pha-ecies-private-key", "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==",
                # These parameters get recorded in Avro messages but otherwise
                # do not affect any system behavior, so the values don't matter.
                "--batch-start-time", "1000000000",
                "--batch-end-time", "1000000100",
                "--dimension", "123",
                "--epsilon", "0.23",
              ]
              env {
                name  = "RUST_LOG"
                value = "info"
              }
              env {
                name  = "RUST_BACKTRACE"
                value = "1"
              }
              env {
                name  = "AWS_ACCOUNT_ID"
                value = data.aws_caller_identity.current.account_id
              }
              # We use the packet decryption key that was generated in this
              # deploy to exercise that key provisioning flow.
              env {
                name = "FACILITATOR_ECIES_PRIVATE_KEY"
                value_from {
                  secret_key_ref {
                    name = var.packet_decryption_key_kubernetes_secret
                    key  = "secret_key"
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}

resource "kubernetes_role" "workflow_manager_role" {
  metadata {
    name      = "${var.environment}-${var.data_share_processor_name}-wfm-role"
    namespace = var.kubernetes_namespace
  }

  rule {
    // API group "" means the core API group.
    api_groups = ["batch", ""]
    // Workflow manager can list pods and jobs.
    // Note: Some of these permissions will probably wind up not being needed.
    // Starting with a moderately generous demonstration set.
    resources = ["namespaces", "pods", "jobs"]
    verbs     = ["get", "list", "watch"]
  }
}

resource "kubernetes_role_binding" "workflow_manager_rolebinding" {
  metadata {
    name      = "${var.environment}-${var.data_share_processor_name}-workflow-manager-can-admin"
    namespace = var.kubernetes_namespace
  }

  role_ref {
    kind      = "Role"
    name      = "${var.environment}-${var.data_share_processor_name}-wfm-role"
    api_group = "rbac.authorization.k8s.io"
  }

  subject {
    kind      = "ServiceAccount"
    name      = module.account_mapping.kubernetes_account_name
    namespace = var.kubernetes_namespace
  }
}

output "service_account_unique_id" {
  value = module.account_mapping.google_service_account_unique_id
}

output "service_account_email" {
  value = module.account_mapping.google_service_account_email
}

output "batch_signing_key" {
  value = kubernetes_secret.batch_signing_key.metadata[0].name
}
