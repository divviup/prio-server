variable "cluster_endpoint" {
  type = string
}

variable "cluster_ca_certificate" {
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

resource "helm_release" "prometheus" {
  name       = "prometheus"
  chart      = "prometheus"
  repository = "https://charts.helm.sh/stable"
  namespace  = kubernetes_namespace.monitoring.metadata[0].name

  set {
    name  = "alertmanager.configFromSecret"
    value = "prometheus-alertmanager-config"
  }
  set {
    name  = "alertmanager.configFileName"
    value = "alertmanager.yml"
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
