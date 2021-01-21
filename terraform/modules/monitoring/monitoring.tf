variable "environment" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "cluster_endpoint" {
  type = string
}

variable "cluster_ca_certificate" {
  type = string
}

variable "prometheus_server_persistent_disk_size_gb" {
  type = string
}

data "google_client_config" "current" {}

provider "helm" {
  kubernetes {
    host                   = var.cluster_endpoint
    cluster_ca_certificate = var.cluster_ca_certificate
    token                  = data.google_client_config.current.access_token
    load_config_file       = false
  }
}

resource "kubernetes_namespace" "monitoring" {
  metadata {
    name = "monitoring"
  }
}

# Prometheus alertmanager must be hooked up to VictorOps to deliver alerts,
# which requires an API key. We don't want to check that key into git, so
# similarly to the batch signing and packet decryption keys, we make a dummy
# k8s secret here and fill in a real value later. See terraform/README.md.
resource "kubernetes_secret" "prometheus_alertmanager_config" {
  metadata {
    name      = "prometheus-alertmanager-config"
    namespace = kubernetes_namespace.monitoring.metadata[0].name
  }

  data = {
    # See comment on batch_signing_key, in modules/kubernetes/kubernetes.tf,
    # about the initial value and the lifecycle block here. The key of the
    # secret map must match "alertmanager.configFileName" in
    # resource.helm_release.prometheus, below.
    "alertmanager.yml" = <<CONFIG
global:
  resolve_timeout: 5m
  http_config: {}
  victorops_api_url: https://alert.victorops.com/integrations/generic/20131114/alert/
route:
  receiver: victorops-receiver
  group_by: ['alertname', 'cluster', 'service']
  group_wait: 10s
  group_interval: 5m
  repeat_interval: 3h
receivers:
- name: victorops-receiver
  victorops_configs:
  - api_key: "not-a-real-key"
    routing_key: "not-a-real-routing-key"
    state_message: 'Alert: {{ .CommonLabels.alertname }}. Summary:{{ .CommonAnnotations.summary }}. RawData: {{ .CommonLabels }}'
templates: []
CONFIG
  }

  lifecycle {
    ignore_changes = [
      data["alertmanager.yml"]
    ]
  }
}

data "google_compute_zones" "available" {}

# Create a GCP persistent disk for use by prometheus-server, with replicas in
# multiple zones. The Prometheus Helm chart can create and manage persistent
# disks on our behalf, but only zonal ones, and we prefer regional disks for
# durability.
resource "google_compute_region_disk" "prometheus_server" {
  name = "${var.environment}-prometheus-server"
  # GCP regional disks support replication across at most two zones
  replica_zones = [data.google_compute_zones.available.names[0], data.google_compute_zones.available.names[1]]
  size          = var.prometheus_server_persistent_disk_size_gb
  region        = var.gcp_region
}

# Create a Kubernetes persistent volume representing the GCP disk
resource "kubernetes_persistent_volume" "prometheus_server" {
  metadata {
    # Kubernetes PVs are _not_ namespaced
    name = "prometheus-server-volume"
  }
  spec {
    access_modes = ["ReadWriteOnce"]
    capacity = {
      storage = "${var.prometheus_server_persistent_disk_size_gb}Gi"
    }
    storage_class_name = "standard"
    volume_mode        = "Filesystem"
    persistent_volume_source {
      gce_persistent_disk {
        fs_type = "ext4"
        pd_name = google_compute_region_disk.prometheus_server.name
      }
    }
  }
}

# Finally, make a Kubernetes persistent volume claim referencing the persistent
# volume
resource "kubernetes_persistent_volume_claim" "prometheus_server" {
  metadata {
    name      = "prometheus-server-claim"
    namespace = kubernetes_namespace.monitoring.metadata[0].name
  }
  spec {
    access_modes       = ["ReadWriteOnce"]
    storage_class_name = "standard"
    resources {
      requests = {
        storage = "${var.prometheus_server_persistent_disk_size_gb}Gi"
      }
    }
    volume_name = kubernetes_persistent_volume.prometheus_server.metadata[0].name
  }
}

resource "helm_release" "prometheus" {
  name       = "prometheus"
  chart      = "prometheus"
  repository = "https://prometheus-community.github.io/helm-charts"
  namespace  = kubernetes_namespace.monitoring.metadata[0].name

  set {
    name  = "alertmanager.configFromSecret"
    value = "prometheus-alertmanager-config"
  }
  set {
    name  = "alertmanager.configFileName"
    value = "alertmanager.yml"
  }
  set {
    name  = "server.persistentVolume.enabled"
    value = "true"
  }
  set {
    name  = "server.persistentVolume.existingClaim"
    value = kubernetes_persistent_volume_claim.prometheus_server.metadata[0].name
  }
  set {
    name  = "server.retention"
    value = "30d"
  }
  # Because we use a persistent volume to store metrics, we cannot use the
  # default RollingUpdate strategy, or we would encounter a deadlock: containers
  # from the new revision cannot become healthy until they mount their volumes,
  # but the volumes are not available until the old revision is terminated, but
  # the old revision can't be terminated until the new revision is healthy. So
  # instead we use the Recreate strategy, which destroys the existing revision
  # before spinning up the new one.
  # https://kubernetes.io/docs/concepts/workloads/controllers/deployment/#strategy
  set {
    name  = "server.strategy.type"
    value = "Recreate"
  }
}

# Configures Grafana to get data from Prometheus. The label on this config map
# must match the value of sidecar.datasources.label in helm_release.grafana for
# the datasource provider sidecar to find it.
resource "kubernetes_config_map" "grafana_datasource_prometheus" {
  metadata {
    name      = "grafana-datasource-prometheus"
    namespace = kubernetes_namespace.monitoring.metadata[0].name
    labels = {
      grafana-datasources = 1
    }
  }

  data = {
    "prometheus-datasource.yml" = <<DATASOURCE
apiVersion: 1
datasources:
- name: prometheus
  type: prometheus
  url: prometheus-server
DATASOURCE
  }
}

resource "helm_release" "grafana" {
  name       = "grafana"
  chart      = "grafana"
  repository = "https://grafana.github.io/helm-charts"
  namespace  = kubernetes_namespace.monitoring.metadata[0].name

  set {
    name  = "sidecar.datasources.enabled"
    value = "true"
  }

  set {
    name  = "sidecar.datasources.label"
    value = "grafana-datasources"
  }
}
