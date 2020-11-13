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
    volume {
      name = "config-volume"
      config_map {
        name = kubernetes_config_map.prometheus_config.metadata[0].name
      }
    }
    volume {
      name = "storage-volume"

      config_map {
        name = kubernetes_persistent_volume_claim.prometheus_disk_claim.metadata[0].name
      }
    }
  }
}

resource "kubernetes_config_map" "prometheus_config" {
  metadata {
    name = "prometheus-config"
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
    name = "prometheus-disk-claim"
  }
  spec {
    access_modes = ["ReadWriteOnce"]
    resources {
      requests = {
        storage = "20Gi"
      }
    }
    storage_class_name = "standard"
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
    storage_class_name               = "standard"
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

resource "kubernetes_service" "grafana" {
  metadata {
    name      = "grafana"
    namespace = "monitor"
  }
  spec {
    selector = {
      app = kubernetes_pod.grafana.metadata.0.labels.app
    }
    port {
      port        = 3000
      target_port = 3000
    }

    type = "LoadBalancer"
  }
}

resource "kubernetes_pod" "grafana" {
  metadata {
    name      = "grafana"
    namespace = "monitor"
    labels = {
      app = "Grafana"
    }
  }

  spec {
    # TODO
    service_account_name = kubernetes_service_account.prometheus_account.metadata[0].name
    container {
      image = "grafana/grafana:latest"
      name  = "grafana"
      args = [
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
    volume {
      name = "config-volume"
      config_map {
        name = kubernetes_config_map.grafana_config.metadata[0].name
      }
    }
    volume {
      name = "storage-volume"

      config_map {
        name = kubernetes_persistent_volume_claim.grafana_disk_claim.metadata[0].name
      }
    }
  }
}

resource "kubernetes_config_map" "grafana_config" {
  metadata {
    name = "grafana-config"
  }

  data = {
    "prometheus.yml" = file("${path.module}/grafana.yml")
  }
}


resource "kubernetes_persistent_volume_claim" "grafana_disk_claim" {
  metadata {
    name = "grafana-disk-claim"
  }
  spec {
    access_modes = ["ReadWriteOnce"]
    resources {
      requests = {
        storage = "20Gi"
      }
    }
    storage_class_name = "standard"
    volume_name        = kubernetes_persistent_volume.grafana_volume.metadata.0.name
  }
}

resource "kubernetes_persistent_volume" "grafana_volume" {
  metadata {
    name = "prio-${var.environment}-grafana-storage"
  }
  spec {
    capacity = {
      storage = "20Gi"
    }
    storage_class_name               = "standard"
    access_modes                     = ["ReadWriteOnce"]
    persistent_volume_reclaim_policy = "Retain"
    persistent_volume_source {
      gce_persistent_disk {
        pd_name = google_compute_disk.grafana_disk.name
      }
    }
  }
}

resource "google_compute_disk" "grafana_disk" {
  name = "prio-${var.environment}-grafana-disk"
  type = "pd-ssd"
  # Hack: We have a region in the vars, but to create a disk we want a zone. Just append "-a"
  zone    = "${var.gcp_region}-a"
  project = var.gcp_project
}


resource "kubernetes_ingress" "prometheus_ingress" {
  metadata {
    name = "prom-ingress"
  }

  spec {
    backend {
      service_name = "prometheus"
      service_port = 9090
    }
  }
}
