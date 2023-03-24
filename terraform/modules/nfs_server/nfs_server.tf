variable "namespace" {
  type = string
}

resource "kubernetes_persistent_volume_claim_v1" "nfs-storage" {
  metadata {
    namespace = var.namespace
    name      = "nfs-storage"
  }
  spec {
    access_modes = ["ReadWriteOnce"]
    resources {
      requests = {
        "storage" = "10Gi"
      }
    }
  }
}

resource "kubernetes_deployment_v1" "nfs-server" {
  metadata {
    namespace = var.namespace
    name      = "nfs-server"
  }
  spec {
    replicas = 1
    selector {
      match_labels = {
        app = "nfs-server"
      }
    }
    template {
      metadata {
        labels = {
          app = "nfs-server"
        }
      }
      spec {
        container {
          name  = "nfs-server"
          image = "registry.k8s.io/volume-nfs:0.8"
          port {
            name           = "nfs"
            container_port = 2049
          }
          port {
            name           = "mountd"
            container_port = 20048
          }
          port {
            name           = "rpcbind"
            container_port = 111
          }
          security_context {
            privileged = true
          }
          volume_mount {
            name       = "storage"
            mount_path = "/exports"
          }
        }
        volume {
          name = "storage"
          persistent_volume_claim {
            claim_name = "nfs-storage"
          }
        }
      }
    }
  }
}

resource "kubernetes_service_v1" "nfs-server" {
  metadata {
    namespace = var.namespace
    name      = "nfs-server"
  }
  spec {
    selector = {
      "app" = "nfs-server"
    }
    port {
      name = "nfs"
      port = 2049
    }
    port {
      name = "mountd"
      port = 20048
    }
    port {
      name = "rpcbind"
      port = 111
    }
  }
}

output "server" {
  value = "${kubernetes_service_v1.nfs-server.metadata[0].name}.${kubernetes_service_v1.nfs-server.metadata[0].namespace}.svc.cluster.local"
}
