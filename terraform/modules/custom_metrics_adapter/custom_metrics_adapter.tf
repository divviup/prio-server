# The custom_metrics_adapter module deploys a Stackdriver Custom Metrics Adapter
# so that GCP metrics are visible to Kubernetes for use in things like
# horizontal pod autoscalers.
#
# References:
# https://cloud.google.com/kubernetes-engine/docs/tutorials/autoscaling-metrics
# This module is adapted from the Google provided Kubernetes yaml:
# https://raw.githubusercontent.com/GoogleCloudPlatform/k8s-stackdriver/master/custom-metrics-stackdriver-adapter/deploy/production/adapter_new_resource_model.yaml

variable "environment" {
  type = string
}

data "kubernetes_namespace" "kube_system" {
  metadata {
    name = "kube-system"
  }
}

resource "kubernetes_namespace" "custom_metrics" {
  metadata {
    name = "custom-metrics"
  }
}

# custom-metrics-stackdriver-adapter needs GCP level permissions to access the
# Stackdriver API, so we make it a GCP service account and map that to a
# Kubernetes service account
module "account_mapping" {
  source                  = "../account_mapping"
  google_account_name     = "${var.environment}-custom-metrics-adapter"
  kubernetes_account_name = "custom-metrics-stackdriver-adapter"
  kubernetes_namespace    = kubernetes_namespace.custom_metrics.metadata[0].name
  environment             = var.environment
}

resource "google_project_iam_member" "stackdriver_exporter" {
  role   = "roles/monitoring.viewer"
  member = "serviceAccount:${module.account_mapping.google_service_account_email}"
}

resource "kubernetes_cluster_role_binding" "auth_delegator" {
  metadata {
    name = "custom-metrics:system:auth-delegator"
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = "system:auth-delegator"
  }
  subject {
    kind      = "ServiceAccount"
    name      = module.account_mapping.kubernetes_account_name
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
  }
}

resource "kubernetes_role_binding" "auth_reader" {
  metadata {
    name      = "custom-metrics-auth-reader"
    namespace = data.kubernetes_namespace.kube_system.metadata[0].name
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "Role"
    name      = "extension-apiserver-authentication-reader"
  }
  subject {
    kind      = "ServiceAccount"
    name      = module.account_mapping.kubernetes_account_name
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
  }
}

resource "kubernetes_cluster_role_binding" "resource_reader" {
  metadata {
    name = "custom-metrics-resource-reader"
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = "view"
  }
  subject {
    kind      = "ServiceAccount"
    name      = module.account_mapping.kubernetes_account_name
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
  }
}

resource "kubernetes_deployment" "stackdriver_adapter" {
  metadata {
    name      = "custom-metrics-stackdriver-adapter"
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    labels = {
      run     = "custom-metrics-stackdriver-adapter"
      k8s-app = "custom-metrics-stackdriver-adapter"
    }
  }
  spec {
    selector {
      match_labels = {
        run     = "custom-metrics-stackdriver-adapter"
        k8s-app = "custom-metrics-stackdriver-adapter"
      }
    }
    template {
      metadata {
        labels = {
          run                             = "custom-metrics-stackdriver-adapter"
          k8s-app                         = "custom-metrics-stackdriver-adapter"
          "kubernetes.io/cluster-service" = "true"
        }
      }
      spec {
        service_account_name            = module.account_mapping.kubernetes_account_name
        automount_service_account_token = true
        container {
          image   = "gcr.io/gke-release/custom-metrics-stackdriver-adapter:v0.12.0-gke.0"
          name    = "pod-custom-metrics-stackdriver-adapter"
          command = ["/adapter", "--use-new-resource-model=true"]
          resources {
            limits {
              cpu    = "0.25"
              memory = "200Mi"
            }
            requests {
              cpu    = "0.25"
              memory = "200Mi"
            }
          }
        }
      }
    }
  }
}

resource "kubernetes_service" "stackdriver_adapter" {
  metadata {
    labels = {
      run                             = "custom-metrics-stackdriver-adapter"
      k8s-app                         = "custom-metrics-stackdriver-adapter"
      "kubernetes.io/cluster-service" = "true"
      "kubernetes.io/name"            = "Adapter"
    }
    name      = "custom-metrics-stackdriver-adapter"
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
  }
  spec {
    selector = {
      run     = "custom-metrics-stackdriver-adapter"
      k8s-app = "custom-metrics-stackdriver-adapter"
    }
    port {
      port        = 443
      target_port = 443
      protocol    = "TCP"
    }
    type = "ClusterIP"
  }
}

resource "kubernetes_api_service" "external_metrics_v1beta1" {
  metadata {
    name = "v1beta1.external.metrics.k8s.io"
  }
  spec {
    service {
      name      = kubernetes_service.stackdriver_adapter.metadata[0].name
      namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    }
    version                  = "v1beta1"
    insecure_skip_tls_verify = true
    group                    = "external.metrics.k8s.io"
    group_priority_minimum   = 100
    version_priority         = 100
  }
}
