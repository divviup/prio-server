variable "gcp_project" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "environment" {
  type = string
}

resource "kubernetes_namespace" "monitor" {
  metadata {
    name = "monitor"
  }
}

resource "kubernetes_service" "prometheus" {
  metadata {
    name      = "prometheus"
    namespace = "monitor"
  }
  spec {
    selector = {
      app = kubernetes_pod.prometheus.metadata.0.labels.app
    }
    port {
      port        = 9090
      target_port = 9090
    }

    type = "LoadBalancer"
  }
}

resource "kubernetes_pod" "prometheus" {
  metadata {
    name      = "prometheus"
    namespace = "monitor"
    labels = {
      app = "Prometheus"
    }
  }

  spec {
    service_account_name = kubernetes_service_account.prometheus_account.metadata[0].name
    container {
      image = "prom/prometheus:latest"
      name  = "prometheus"
      args = [
        "--config.file", "/config/prometheus.yml",
        "--storage.tsdb.path", "/data"
      ]
      volume_mount {
        name       = "config-volume"
        mount_path = "/config"
      }
      volume_mount {
        name       = "storage-volume"
        mount_path = "/data"
      }
    }
    security_context {
      fs_group        = 2000
      run_as_user     = 1000
      run_as_non_root = true
    }
    volume {
      name = "config-volume"
      config_map {
        name = kubernetes_config_map.prometheus_config.metadata[0].name
      }
    }
    volume {
      name = "storage-volume"

      persistent_volume_claim {
        claim_name = kubernetes_persistent_volume_claim.prometheus_disk_claim.metadata[0].name
      }
    }
  }
}

resource "kubernetes_config_map" "prometheus_config" {
  metadata {
    name      = "prometheus-config"
    namespace = "monitor"
  }

  data = {
    "prometheus.yml" = file("${path.module}/prometheus.yml")
  }
}

resource "kubernetes_cluster_role" "prometheus_cluster_role" {
  metadata {
    name = "prometheus-role"
  }

  rule {
    // API group "" means the core API group.
    api_groups = [""]
    resources  = ["nodes", "services", "endpoints", "pods"]
    verbs      = ["get", "list", "watch"]
  }
  rule {
    api_groups = [""]
    resources  = ["configmaps"]
    verbs      = ["get"]
  }
}

resource "kubernetes_service_account" "prometheus_account" {
  metadata {
    name      = "prometheus-account"
    namespace = "monitor"
  }
}

resource "kubernetes_cluster_role_binding" "prometheus_binding" {
  metadata {
    name = "prometheus_binding"
  }

  role_ref {
    kind      = "ClusterRole"
    name      = "prometheus_cluster_role"
    api_group = "rbac.authorization.k8s.io"
  }

  subject {
    kind      = "ServiceAccount"
    name      = "prometheus-account"
    namespace = "monitor"
  }
}

resource "kubernetes_persistent_volume_claim" "prometheus_disk_claim" {
  metadata {
    name      = "prometheus-disk-claim"
    namespace = "monitor"
  }
  spec {
    access_modes = ["ReadWriteOnce"]
    resources {
      requests = {
        storage = "5Gi"
      }
    }
    storage_class_name = "gce"
    volume_name        = kubernetes_persistent_volume.prometheus_volume.metadata.0.name
  }
}

resource "kubernetes_persistent_volume" "prometheus_volume" {
  metadata {
    name = "prio-${var.environment}-prometheus-storage"
  }
  spec {
    capacity = {
      storage = "20Gi"
    }
    storage_class_name               = "gce"
    access_modes                     = ["ReadWriteOnce"]
    persistent_volume_reclaim_policy = "Retain"
    persistent_volume_source {
      gce_persistent_disk {
        pd_name = google_compute_disk.prometheus_disk.name
      }
    }
  }
}

resource "google_compute_disk" "prometheus_disk" {
  name = "prio-${var.environment}-prometheus-disk"
  type = "pd-ssd"
  # Hack: We have a region in the vars, but to create a disk we want a zone. Just append "-a"
  zone    = "${var.gcp_region}-a"
  project = var.gcp_project
}
