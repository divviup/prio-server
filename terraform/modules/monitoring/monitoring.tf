variable "gcp_project" {
  type = string
}

variable "gcp_region" {
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
    }
    volume {
      name = "config-volume"
      config_map {
        name = "prometheus_config"
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
    #namespace = "monitor"
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

resource "kubernetes_persistent_volume" "prometheus_storage" {
  metadata {
    name = "prometheus-storage"
  }
  spec {
    capacity = {
      storage = "20Gi"
    }
    access_modes                     = ["ReadWriteOnce"]
    persistent_volume_reclaim_policy = "Retain"
    persistent_volume_source {
      gce_persistent_disk {
        pd_name = google_compute_disk.prom_store.name
      }
    }
  }
}

resource "google_compute_disk" "prom_store" {
  name = "prom-store"
  type = "pd-ssd"
  # Hack: We have a region in the vars, but to create a disk we want a zone. Just append "-a"
  zone    = "${var.gcp_region}-a"
  project = var.gcp_project
}
