# Set up FluentBit so that cluster and application logs go to CloudWatch Logs.
# The resources here are translated from the Kubernetes YAML provided in AWS
# documentation:
# https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/Container-Insights-setup-logs-FluentBit.html

# Namespace in which the FluentBit daemonset runs
resource "kubernetes_namespace" "fluent_bit" {
  metadata {
    name = "fluent-bit"
  }
}

# ConfigMap containing EKS cluster info. All of this could go into the
# fluent-bit-config ConfigMap but this keeps us in line with Amazon's guide.
resource "kubernetes_config_map" "cluster_info" {
  metadata {
    name      = "fluent-bit-cluster-info"
    namespace = kubernetes_namespace.fluent_bit.metadata[0].name
  }

  data = {
    "cluster.name" = data.aws_eks_cluster.cluster.name
    "http.server"  = "Off"
    "http.port"    = "2020"
    "read.head"    = "Off"
    "read.tail"    = "On"
    "logs.region"  = var.aws_region
  }
}

# Configuration for FluentBit
resource "kubernetes_config_map" "fluent_bit" {
  metadata {
    name      = "fluent-bit-config"
    namespace = kubernetes_namespace.fluent_bit.metadata[0].name
    labels = {
      k8s-app = "fluent-bit"
    }
  }

  data = {
    "fluent-bit.conf"      = file("${path.module}/config/fluent-bit.conf")
    "application-log.conf" = file("${path.module}/config/application-log.conf")
    "dataplane-log.conf"   = file("${path.module}/config/dataplane-log.conf")
    "host-log.conf"        = file("${path.module}/config/host-log.conf")
    "parsers.conf"         = file("${path.module}/config/parsers.conf")
  }
}

resource "kubernetes_service_account" "fluent_bit" {
  metadata {
    name      = "fluent-bit"
    namespace = kubernetes_namespace.fluent_bit.metadata[0].name
  }
}

resource "kubernetes_cluster_role" "fluent_bit" {
  metadata {
    name = "fluent-bit"
  }

  rule {
    api_groups = [""]
    resources  = ["namespaces", "pods", "pods/logs"]
    verbs      = ["get", "list", "watch"]
  }

  rule {
    non_resource_urls = ["/metrics"]
    verbs             = ["get"]
  }
}

resource "kubernetes_cluster_role_binding" "fluent_bit" {
  metadata {
    name = "fluent-bit"
  }

  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = kubernetes_cluster_role.fluent_bit.metadata[0].name
  }

  subject {
    kind      = "ServiceAccount"
    name      = kubernetes_service_account.fluent_bit.metadata[0].name
    namespace = kubernetes_namespace.fluent_bit.metadata[0].name
  }
}

resource "kubernetes_daemonset" "fluent_bit" {
  metadata {
    name      = "fluent-bit"
    namespace = kubernetes_namespace.fluent_bit.metadata[0].name
    labels = {
      k8s-app                         = "fluent-bit"
      version                         = "v1"
      "kubernetes.io/cluster-service" = "true"
    }
  }

  spec {
    selector {
      match_labels = {
        k8s-app = "fluent-bit"
      }
    }

    template {
      metadata {
        labels = {
          k8s-app                         = "fluent-bit"
          version                         = "v1"
          "kubernetes.io/cluster-service" = "true"
        }
      }
      spec {
        termination_grace_period_seconds = 10
        service_account_name             = "fluent-bit"

        volume {
          name = "fluentbitstate"
          host_path {
            path = "/var/fluent-bit/state"
          }
        }
        volume {
          name = "varlog"
          host_path {
            path = "/var/log"
          }
        }
        volume {
          name = "varlibdockercontainers"
          host_path {
            path = "/var/lib/docker/containers"
          }
        }
        volume {
          name = "fluent-bit-config"
          config_map {
            name = "fluent-bit-config"
          }
        }
        volume {
          name = "runlogjournal"
          host_path {
            path = "/run/log/journal"
          }
        }
        volume {
          name = "dmesg"
          host_path {
            path = "/var/log/dmesg"
          }
        }

        toleration {
          key      = "node-role.kubernetes.io/master"
          operator = "Exists"
          effect   = "NoSchedule"
        }
        toleration {
          operator = "Exists"
          effect   = "NoExecute"
        }
        toleration {
          operator = "Exists"
          effect   = "NoSchedule"
        }

        container {
          name              = "fluent-bit"
          image             = "amazon/aws-for-fluent-bit:2.10.0"
          image_pull_policy = "Always"

          env {
            name = "AWS_REGION"
            value_from {
              config_map_key_ref {
                name = kubernetes_config_map.cluster_info.metadata[0].name
                key  = "logs.region"
              }
            }
          }
          env {
            name = "CLUSTER_NAME"
            value_from {
              config_map_key_ref {
                name = kubernetes_config_map.cluster_info.metadata[0].name
                key  = "cluster.name"
              }
            }
          }
          env {
            name = "HTTP_SERVER"
            value_from {
              config_map_key_ref {
                name = kubernetes_config_map.cluster_info.metadata[0].name
                key  = "http.server"
              }
            }
          }
          env {
            name = "HTTP_PORT"
            value_from {
              config_map_key_ref {
                name = kubernetes_config_map.cluster_info.metadata[0].name
                key  = "http.port"
              }
            }
          }
          env {
            name = "READ_FROM_HEAD"
            value_from {
              config_map_key_ref {
                name = kubernetes_config_map.cluster_info.metadata[0].name
                key  = "read.head"
              }
            }
          }
          env {
            name = "READ_FROM_TAIL"
            value_from {
              config_map_key_ref {
                name = kubernetes_config_map.cluster_info.metadata[0].name
                key  = "read.tail"
              }
            }
          }
          env {
            name = "HOST_NAME"
            value_from {
              field_ref {
                field_path = "spec.nodeName"
              }
            }
          }
          env {
            name  = "CI_VERSION"
            value = "k8s/1.3.8"
          }

          resources {
            requests = {
              memory = "100Mi"
              cpu    = "500m"
            }
            limits = {
              memory = "200Mi"
            }
          }

          volume_mount {
            name       = "fluentbitstate"
            mount_path = "/var/fluent-bit/state"
          }
          volume_mount {
            name       = "varlog"
            mount_path = "/var/log"
            read_only  = true
          }

          volume_mount {
            name       = "varlibdockercontainers"
            mount_path = "/var/lib/docker/containers"
            read_only  = true
          }

          volume_mount {
            name       = "fluent-bit-config"
            mount_path = "/fluent-bit/etc/"
          }

          volume_mount {
            name       = "runlogjournal"
            mount_path = "/run/log/journal"
            read_only  = true
          }

          volume_mount {
            name       = "dmesg"
            mount_path = "/var/log/dmesg"
            read_only  = true
          }
        }
      }
    }
  }
}
