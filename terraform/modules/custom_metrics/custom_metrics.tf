variable "environment" {
  type = string
}

variable "use_aws" {
  type    = bool
  default = false
}

data "google_project" "project" {}

resource "kubernetes_namespace" "custom_metrics" {
  metadata {
    name = "custom-metrics"
  }
}

module "account_mapping" {
  source                          = "../account_mapping"
  kubernetes_service_account_name = "custom-metrics-adapter"
  kubernetes_namespace            = kubernetes_namespace.custom_metrics.metadata[0].name
  environment                     = var.environment
  aws_iam_role_name               = var.use_aws ? "${var.environment}-metrics-adapter" : ""
  gcp_service_account_name        = var.use_aws ? "" : "${var.environment}-metrics-adapter"
  gcp_project                     = data.google_project.project.project_id
}

resource "kubernetes_cluster_role_binding" "custom_metrics_adapter_auth_delegator" {
  metadata {
    name = "custom-metrics-adapter:system:auth-delegator"
  }
  role_ref {
    kind      = "ClusterRole"
    api_group = "rbac.authorization.k8s.io"
    name      = "system:auth-delegator"
  }
  subject {
    kind      = "ServiceAccount"
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = module.account_mapping.kubernetes_service_account_name
  }
}

resource "kubernetes_role_binding" "custom_metrics_adapter_extension_apiserver_authentication_reader" {
  metadata {
    namespace = "kube-system"
    name      = "custom-metrics-adapter:extension-apiserver-authentication-reader"
  }
  role_ref {
    kind      = "Role"
    api_group = "rbac.authorization.k8s.io"
    name      = "extension-apiserver-authentication-reader"
  }
  subject {
    kind      = "ServiceAccount"
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = module.account_mapping.kubernetes_service_account_name
  }
}

resource "kubernetes_cluster_role_binding" "horizontal_pod_autoscaler_external_metrics_reader" {
  metadata {
    name = "horizontal-pod-autoscaler:external-metrics-reader"
  }
  role_ref {
    kind      = "ClusterRole"
    api_group = "rbac.authorization.k8s.io"
    name      = "external-metrics-reader"
  }
  subject {
    kind      = "ServiceAccount"
    namespace = "kube-system"
    name      = "horizontal-pod-autoscaler"
  }
}

resource "kubernetes_api_service" "external_metrics" {
  metadata {
    name = "v1beta1.external.metrics.k8s.io"
  }
  spec {
    group                  = "external.metrics.k8s.io"
    version                = "v1beta1"
    group_priority_minimum = 100
    version_priority       = 100
    service {
      namespace = kubernetes_namespace.custom_metrics.metadata[0].name
      name      = "custom-metrics-adapter"
    }
    insecure_skip_tls_verify = true # XXX: is this OK???
  }
}


##
## GCP-specific resources
##
resource "google_project_iam_member" "monitoring_viewer" {
  count   = var.use_aws ? 0 : 1
  project = data.google_project.project.project_id
  role    = "roles/monitoring.viewer"
  member  = "serviceAccount:${module.account_mapping.gcp_service_account_email}"
}

resource "kubernetes_cluster_role_binding" "custom_metrics_adapter_view" {
  count = var.use_aws ? 0 : 1
  metadata {
    name = "custom-metrics-adapter:view"
  }
  role_ref {
    kind      = "ClusterRole"
    api_group = "rbac.authorization.k8s.io"
    name      = "view"
  }
  subject {
    kind      = "ServiceAccount"
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = module.account_mapping.kubernetes_service_account_name
  }
}

resource "kubernetes_deployment" "gcp_custom_metrics_adapter" {
  count = var.use_aws ? 0 : 1
  metadata {
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = "custom-metrics-adapter"
    labels    = { app = "custom-metrics-adapter" } # XXX: is this label/selector strategy sufficient? GCP and AWS have different implementations (but I think the difference is arbitrary)
  }
  spec {
    replicas = 1
    selector { match_labels = { app = "custom-metrics-adapter" } }
    template {
      metadata {
        name   = "custom-metrics-adapter"
        labels = { app = "custom-metrics-adapter" }
      }
      spec {
        service_account_name = module.account_mapping.kubernetes_service_account_name
        container {
          name              = "custom-metrics-adapter"
          image             = "gcr.io/gke-release/custom-metrics-stackdriver-adapter:v0.12.0-gke.0"
          image_pull_policy = "Always"
          command           = ["/adapter", "--use-new-resource-model=false"]
          resources {
            requests = {
              cpu    = "250m"
              memory = "200Mi"
            }
            limits = {
              cpu    = "250m"
              memory = "200Mi"
            }
          }
        }
      }
    }
  }
}

resource "kubernetes_service" "gcp_custom_metrics_adapter" {
  count = var.use_aws ? 0 : 1
  metadata {
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = "custom-metrics-adapter"
    labels    = { app = "custom-metrics-adapter" }
  }
  spec {
    port {
      port        = 443
      protocol    = "TCP"
      target_port = 443
    }
    selector = { app = "custom-metrics-adapter" }
    type     = "ClusterIP"
  }
}

resource "kubernetes_api_service" "custom_metrics_v1beta1" {
  count = var.use_aws ? 0 : 1
  metadata {
    name = "v1beta1.custom.metrics.k8s.io"
  }
  spec {
    group                  = "custom.metrics.k8s.io"
    version                = "v1beta1"
    group_priority_minimum = 100
    version_priority       = 100
    service {
      namespace = kubernetes_namespace.custom_metrics.metadata[0].name
      name      = "custom-metrics-adapter"
    }
    insecure_skip_tls_verify = true # XXX: is this OK???
  }
}

resource "kubernetes_api_service" "custom_metrics_v1beta2" {
  count = var.use_aws ? 0 : 1
  metadata {
    name = "v1beta2.custom.metrics.k8s.io"
  }
  spec {
    group                  = "custom.metrics.k8s.io"
    version                = "v1beta2"
    group_priority_minimum = 100
    version_priority       = 200
    service {
      namespace = kubernetes_namespace.custom_metrics.metadata[0].name
      name      = "custom-metrics-adapter"
    }
    insecure_skip_tls_verify = true # XXX: is this OK???
  }
}


##
## AWS-specific resources
##
resource "kubernetes_cluster_role" "aws_metrics_reader" {
  count = var.use_aws ? 1 : 0
  metadata {
    name = "aws-metrics-reader"
  }
  rule {
    api_groups = ["metrics.aws"]
    resources  = ["externalmetrics"]
    verbs      = ["list", "get", "watch"]
  }
}

resource "kubernetes_cluster_role_binding" "custom_metrics_adapter_aws_metrics_reader" {
  count = var.use_aws ? 1 : 0
  metadata {
    name = "custom-metrics-adapter:aws_metrics_reader"
  }
  role_ref {
    kind      = "ClusterRole"
    api_group = "rbac.authorization.k8s.io"
    name      = kubernetes_cluster_role.aws_metrics_reader[0].metadata[0].name
  }
  subject {
    kind      = "ServiceAccount"
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = module.account_mapping.kubernetes_service_account_name
  }
}

resource "kubernetes_cluster_role" "resource_reader" {
  count = var.use_aws ? 1 : 0
  metadata {
    name = "resource-reader"
  }
  rule {
    api_groups = [""]
    resources  = ["namespaces", "pods", "services", "configmaps"]
    verbs      = ["get", "list"]
  }
}

resource "kubernetes_cluster_role_binding" "custom_metrics_adapter_resource_reader" {
  count = var.use_aws ? 1 : 0
  metadata {
    name = "custom-metrics-adapter:resource-reader"
  }
  role_ref {
    kind      = "ClusterRole"
    api_group = "rbac.authorization.k8s.io"
    name      = kubernetes_cluster_role.resource_reader[0].metadata[0].name
  }
  subject {
    kind      = "ServiceAccount"
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = module.account_mapping.kubernetes_service_account_name
  }
}

resource "kubernetes_deployment" "aws_custom_metrics_adapter" {
  count = var.use_aws ? 1 : 0
  metadata {
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = "custom-metrics-adapter"
    labels    = { app = "custom-metrics-adapter" }
  }
  spec {
    replicas = 1
    selector { match_labels = { app = "custom-metrics-adapter" } }
    template {
      metadata {
        name   = "custom-metrics-adapter"
        labels = { app = "custom-metrics-adapter" }
      }
      spec {
        service_account_name = module.account_mapping.kubernetes_service_account_name
        security_context { fs_group = 65534 } # XXX: what is 65534?
        container {
          name              = "custom-metrics-adapter"
          image             = "chankh/k8s-cloudwatch-adapter:v0.10.0"
          image_pull_policy = "Always"
          command           = ["/adapter", "--cert-dir=/tmp", "--secure-port=6443", "--logtostderr=true", "--v=2"]
          port {
            container_port = 6443 # XXX: can this just be 443 to simplify service definition?
            name           = "https"
          }
          port {
            container_port = 8080
            name           = "http"
          }
          volume_mount {
            name       = "temp-vol"
            mount_path = "/tmp"
          }
        }
        volume {
          name = "temp-vol"
          empty_dir {}
        }
      }
    }
  }
}

resource "kubernetes_service" "aws_custom_metrics_adapter" {
  count = var.use_aws ? 1 : 0
  metadata {
    namespace = kubernetes_namespace.custom_metrics.metadata[0].name
    name      = "custom-metrics-adapter"
    labels    = { app = "custom-metrics-adapter" }
  }
  spec {
    port {
      name        = "https"
      port        = 443
      target_port = 6443
    }
    port {
      name        = "http"
      port        = 80
      target_port = 8080
    }
    selector = { app = "custom-metrics-adapter" }
  }
}