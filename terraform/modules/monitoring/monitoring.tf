variable "environment" {
  type = string
}

variable "use_aws" {
  type = bool
}

variable "gcp" {
  type = object({
    region  = string
    project = string
  })
}

variable "eks_oidc_provider" {
  type = object({
    arn = string
    url = string
  })
  default = {
    arn = ""
    url = ""
  }
}

variable "prometheus_server_persistent_disk_size_gb" {
  type = string
}

variable "victorops_routing_key" {
  type = string
}

variable "aggregation_period" {
  type = string
}

variable "prometheus_helm_chart_version" {
  type = string
}

variable "grafana_helm_chart_version" {
  type = string
}

variable "cloudwatch_exporter_helm_chart_version" {
  type = string
}

variable "stackdriver_exporter_helm_chart_version" {
  type = string
}

terraform {
  required_providers {
    helm = {
      source  = "hashicorp/helm"
      version = "2.10.1"
    }
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
  - api_key: "not-a-real-api-key"
    routing_key: "${var.victorops_routing_key}"
    # This is the default value for entity_display_name, made explicit.
    entity_display_name: '{{ template "victorops.default.entity_display_name" . }}'
    state_message: |-
      Summary: {{ template "pretty-summary" . }}.
      Description: {{ template "pretty-description" . }}
      Common Labels: {{ .CommonLabels }}
      Per-Alert Labels:
      {{- $commonLabelsKeys := .CommonLabels.Names }}
      {{ range .Alerts.Firing }}{{ .Labels.Remove $commonLabelsKeys }}
      {{ end }}
templates:
- "/etc/config/templates.d/annotations.tmpl"
CONFIG
  }

  lifecycle {
    ignore_changes = [
      data["alertmanager.yml"]
    ]
  }
}

resource "kubernetes_config_map_v1" "prometheus_alertmanager_templates" {
  metadata {
    name      = "prometheus-alertmanager-templates"
    namespace = kubernetes_namespace.monitoring.metadata[0].name
  }

  data = {
    "annotations.tmpl" = <<TEMPLATE
{{- define "pretty-summary" -}}
  {{- if .CommonAnnotations.summary -}}
    {{- .CommonAnnotations.summary -}}
  {{- else -}}
    {{- if gt (len .Alerts.Firing) 0 -}}
      {{- with index .Alerts.Firing 0 -}}
        {{- .Annotations.summary -}}
      {{- end -}}
      {{- $otherCount := slice .Alerts.Firing 1 | len -}}
      {{- if $otherCount }} (and {{ $otherCount }} other{{ if ne $otherCount 1 }}s{{ end }}){{ end -}}
    {{- else if gt (len .Alerts.Resolved) 0 -}}
      {{- with index .Alerts.Resolved 0 -}}
        {{- .Annotations.summary -}}
      {{- end -}}
      {{- $otherCount := slice .Alerts.Resolved 1 | len -}}
      {{- if $otherCount }} (and {{ $otherCount }} other{{ if ne $otherCount 1 }}s{{ end }}){{ end -}}
    {{- end -}}
  {{- end -}}
{{- end -}}
{{- define "pretty-description" -}}
  {{- if .CommonAnnotations.description -}}
    {{- .CommonAnnotations.description -}}
  {{- else -}}
    {{- if gt (len .Alerts.Firing) 0 -}}
      {{- with index .Alerts.Firing 0 -}}
        {{- .Annotations.description -}}
      {{- end -}}
      {{- $otherCount := slice .Alerts.Firing 1 | len -}}
      {{- if $otherCount }} (and {{ $otherCount }} other{{ if ne $otherCount 1 }}s{{ end }}){{ end -}}
    {{- else if gt (len .Alerts.Resolved) 0 -}}
      {{- with index .Alerts.Resolved 0 -}}
        {{- .Annotations.description -}}
      {{- end -}}
      {{- $otherCount := slice .Alerts.Resolved 1 | len -}}
      {{- if $otherCount }} (and {{ $otherCount }} other{{ if ne $otherCount 1 }}s{{ end }}){{ end -}}
    {{- end -}}
  {{- end -}}
{{- end -}}
TEMPLATE
  }
}

data "google_compute_zones" "available" {
  count = var.use_aws ? 0 : 1
}

# Create a GCP persistent disk for use by prometheus-server, with replicas in
# multiple zones. The Prometheus Helm chart can create and manage persistent
# disks on our behalf, but only zonal ones, and we prefer regional disks for
# durability.
resource "google_compute_region_disk" "prometheus_server" {
  count = var.use_aws ? 0 : 1
  name  = "${var.environment}-prometheus-server"
  # GCP regional disks support replication across at most two zones
  replica_zones = [data.google_compute_zones.available[0].names[0], data.google_compute_zones.available[0].names[1]]
  size          = var.prometheus_server_persistent_disk_size_gb
  region        = var.gcp.region

  lifecycle {
    ignore_changes = [labels]
  }
}

# Create a Kubernetes persistent volume representing the GCP disk
resource "kubernetes_persistent_volume" "prometheus_server_gcp" {
  count = var.use_aws ? 0 : 1
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
        pd_name = google_compute_region_disk.prometheus_server[0].name
      }
    }
  }
}

data "aws_availability_zones" "available" {
  count = var.use_aws ? 1 : 0
  state = "available"
}

# Create an AWS EBS volume for use by prometheus-server. EBS volumes are zonal,
# which means we don't get the redundancy that GCP disks provide. It also means
# that the prometheus-server pod _must_ be scheduled onto a node in the same AZ
# as the volume, and thus that the cluster must have at least one node in that
# AZ.
resource "aws_ebs_volume" "prometheus_server" {
  count             = var.use_aws ? 1 : 0
  availability_zone = data.aws_availability_zones.available[0].names[0]
  size              = var.prometheus_server_persistent_disk_size_gb
  tags = {
    Name = "${var.environment}-prometheus-server"
  }
}

# Create a Kubernetes persistent volume representing the Elastic Block Store
# volume
resource "kubernetes_persistent_volume" "prometheus_server_aws" {
  count = var.use_aws ? 1 : 0
  metadata {
    name = "prometheus-server-volume"
  }
  spec {
    access_modes = ["ReadWriteOnce"]
    capacity = {
      storage = "${var.prometheus_server_persistent_disk_size_gb}Gi"
    }
    storage_class_name = "standard"
    volume_mode        = "Filesystem"
    # We put a node affinity on the PV to ensure that prometheus-server gets
    # scheduled onto a node in the AZ where the EBS volume is. This is done
    # automatically for gce_persistent_disk persistent volumes, but we have to
    # do it ourselves for EBS.
    node_affinity {
      required {
        node_selector_term {
          match_expressions {
            key      = "failure-domain.beta.kubernetes.io/zone"
            operator = "In"
            values   = [aws_ebs_volume.prometheus_server[0].availability_zone]
          }
          match_expressions {
            key      = "failure-domain.beta.kubernetes.io/region"
            operator = "In"
            # Per Terraform docs, id is "Region of the Availability Zones"
            values = [data.aws_availability_zones.available[0].id]
          }
        }
      }
    }
    persistent_volume_source {
      aws_elastic_block_store {
        fs_type   = "ext4"
        volume_id = aws_ebs_volume.prometheus_server[0].id
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
    volume_name = var.use_aws ? (
      kubernetes_persistent_volume.prometheus_server_aws[0].metadata[0].name
      ) : (
      kubernetes_persistent_volume.prometheus_server_gcp[0].metadata[0].name
    )
  }
}

# Metrics are stored in Prometheus. Documentation and config reference:
# https://artifacthub.io/packages/helm/prometheus-community/prometheus
resource "helm_release" "prometheus" {
  name       = "prometheus"
  chart      = "prometheus"
  repository = "https://prometheus-community.github.io/helm-charts"
  namespace  = kubernetes_namespace.monitoring.metadata[0].name
  version    = var.prometheus_helm_chart_version

  set {
    name  = "alertmanager.configFromSecret"
    value = kubernetes_secret.prometheus_alertmanager_config.metadata[0].name
  }
  set {
    name  = "alertmanager.extraConfigmapMounts[0].name"
    value = "template-files"
  }
  set {
    name  = "alertmanager.extraConfigmapMounts[0].mountPath"
    value = "/etc/config/templates.d"
  }
  set {
    name  = "alertmanager.extraConfigmapMounts[0].configMap"
    value = kubernetes_config_map_v1.prometheus_alertmanager_templates.metadata[0].name
  }
  set {
    name  = "alertmanager.extraConfigmapMounts[0].readOnly"
    value = "true"
  }
  set {
    name  = "configmapReload.alertmanager.extraConfigmapMounts[0].name"
    value = "template-files"
  }
  set {
    name  = "configmapReload.alertmanager.extraConfigmapMounts[0].mountPath"
    value = "/etc/config/templates.d"
  }
  set {
    name  = "configmapReload.alertmanager.extraConfigmapMounts[0].configMap"
    value = kubernetes_config_map_v1.prometheus_alertmanager_templates.metadata[0].name
  }
  set {
    name  = "configmapReload.alertmanager.extraConfigmapMounts[0].readOnly"
    value = "true"
  }
  set {
    name  = "configmapReload.alertmanager.extraVolumeDirs[0]"
    value = "/etc/config/templates.d"
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
  set {
    name  = "alertmanager.strategy.type"
    value = "Recreate"
  }

  # Alerting rules are defined in their own YAML file. We then need to provide
  # them to the Helm chart, under the key serverFiles."alerting_rules.yml".
  # To ensure we get a properly nested and indented YAML structure and that
  # "alerting_rules.yml" doesn't get expanded into a map "alerting_rules"
  # containing the key "yml", we decode the alerting rules into a Terraform map
  # and then encode the entire map back into YAML, then stick that into the
  # chart.
  values = [
    yamlencode({
      serverFiles = {
        "alerting_rules.yml" = yamldecode(
          templatefile("${path.module}/prometheus_alerting_rules.yml", {
            environment        = var.environment
            aggregation_period = var.aggregation_period
          })
        )
      }
    })
  ]

  # Prometheus v2.37 is a LTS release.
  # See https://prometheus.io/docs/introduction/release-cycle/
  set {
    name  = "server.image.tag"
    value = "v2.37.5"
  }
  # See https://github.com/prometheus/alertmanager/releases
  set {
    name  = "alertmanager.image.tag"
    value = "v0.25.0"
  }
  # See https://github.com/prometheus/node_exporter/releases
  set {
    name  = "nodeExporter.image.tag"
    value = "v1.5.0"
  }
  # See https://github.com/prometheus/pushgateway/releases
  set {
    name  = "pushgateway.image.tag"
    value = "v1.5.1"
  }

  # Pull from registry.k8s.io instead of k8s.gcr.io
  set {
    name  = "kube-state-metrics.image.repository"
    value = "registry.k8s.io/kube-state-metrics/kube-state-metrics"
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
  url: http://prometheus-server
DATASOURCE
  }
}

resource "kubernetes_config_map" "grafana_dashboard_default" {
  metadata {
    name      = "grafana-dashboard-default"
    namespace = kubernetes_namespace.monitoring.metadata[0].name
    labels = {
      grafana-dashboard = 1
    }
  }

  data = {
    "default_dashboard.json"        = file("${path.module}/default_dashboard.json")
    "resource_usage_dashboard.json" = file("${path.module}/resource_usage_dashboard.json")
    "http_api_dashboard.json"       = file("${path.module}/http_api_dashboard.json")
  }
}

# We use Grafana as a frontend to Prometheus for dashboards. Documentation and
# config reference: https://artifacthub.io/packages/helm/grafana/grafana
resource "helm_release" "grafana" {
  name       = "grafana"
  chart      = "grafana"
  repository = "https://grafana.github.io/helm-charts"
  namespace  = kubernetes_namespace.monitoring.metadata[0].name
  version    = var.grafana_helm_chart_version

  set {
    name  = "sidecar.datasources.enabled"
    value = "true"
  }

  set {
    name  = "sidecar.datasources.label"
    value = "grafana-datasources"
  }

  set {
    name  = "sidecar.dashboards.enabled"
    value = "true"
  }

  set {
    name  = "sidecar.dashboards.label"
    value = "grafana-dashboard"
  }
}

# The Prometheus CloudWatch Exporter forwards metrics from AWS CloudWatch into
# Prometheus. Documentation and config reference:
# https://github.com/prometheus/cloudwatch_exporter
# https://artifacthub.io/packages/helm/prometheus-community/prometheus-cloudwatch-exporter
# https://github.com/prometheus-community/helm-charts/tree/main/charts/prometheus-cloudwatch-exporter
resource "helm_release" "cloudwatch_exporter" {
  count      = var.use_aws ? 1 : 0
  name       = "cloudwatch-exporter"
  chart      = "prometheus-cloudwatch-exporter"
  repository = "https://prometheus-community.github.io/helm-charts"
  namespace  = kubernetes_namespace.monitoring.metadata[0].name
  version    = var.cloudwatch_exporter_helm_chart_version

  set {
    name  = "serviceAccount.create"
    value = false
  }

  set {
    name  = "serviceAccount.name"
    value = module.account_mapping.kubernetes_service_account_name
  }

  set {
    name = "config"
    value = yamlencode({
      # Individual metrics to be forwarded to Prometheus must be enumerated here
      metrics = [
        # SQS queue depth
        {
          aws_namespace          = "AWS/SQS"
          aws_metric_name        = "ApproximateNumberOfMessagesVisible"
          aws_dimensions         = ["QueueName"]
          list_metrics_cache_ttl = 600 # 10 minutes
        }
      ]
    })
  }

  values = [
    <<VALUE
service:
  annotations:
    prometheus.io/scrape: "true"
VALUE
  ]
}

# The Stackdriver Prometheus exporter forwards metrics from Google Cloud
# Monitoring (aka Stackdriver) into Prometheus. Documentation and config
# reference:
# https://github.com/prometheus-community/stackdriver_exporter
# https://artifacthub.io/packages/helm/prometheus-community/prometheus-stackdriver-exporter
# https://github.com/prometheus-community/helm-charts/tree/main/charts/prometheus-stackdriver-exporter
resource "helm_release" "stackdriver_exporter" {
  count      = var.use_aws ? 0 : 1
  name       = "stackdriver-exporter"
  chart      = "prometheus-stackdriver-exporter"
  repository = "https://prometheus-community.github.io/helm-charts"
  namespace  = kubernetes_namespace.monitoring.metadata[0].name
  version    = var.stackdriver_exporter_helm_chart_version

  set {
    name  = "stackdriver.projectId"
    value = var.gcp.project
  }

  set {
    name  = "serviceAccount.name"
    value = module.account_mapping.kubernetes_service_account_name
  }

  set {
    name = "stackdriver.metrics.typePrefixes"
    # The metrics to be forwarded from Stackdriver to Prometheus. The prefix
    # list is comma separated, and we have to escape the comma to avoid
    # confusing Helm:
    # https://github.com/hashicorp/terraform-provider-helm/issues/226
    value = join("\\,", [
      # PubSub metrics
      # https://cloud.google.com/monitoring/api/metrics_gcp#gcp-pubsub
      "pubsub.googleapis.com/subscription/",
      "pubsub.googleapis.com/topic/",
    ])
  }

  # We must annotate the stackdriver exporter Kubernetes service to be
  # discovered by Prometheus as a scrape target. It is surprisingly hard to get
  # this represented just right so that the transformation from Terraform value
  # to Helm argument to Kubernetes annotation comes out right, hence the YAML
  # literal heredoc.
  # https://github.com/hashicorp/terraform-provider-helm/issues/64
  values = [
    <<VALUE
service:
  annotations:
    prometheus.io/scrape: "true"
VALUE
  ]
}

# stackdriver-exporter or cloudwatch-exporter need some GCP or AWS level
# permissions to access the respective metrics APIs, so we make them a GCP
# service account or AWS IAM role as appropriate
# https://github.com/prometheus-community/stackdriver_exporter#credentials-and-permissions
module "account_mapping" {
  source                          = "../account_mapping"
  gcp_service_account_name        = var.use_aws ? "" : "${var.environment}-stackdriver-exporter"
  gcp_project                     = var.gcp.project
  aws_iam_role_name               = var.use_aws ? "${var.environment}-stackdriver-exporter" : ""
  eks_oidc_provider               = var.eks_oidc_provider
  kubernetes_service_account_name = "stackdriver-exporter"
  kubernetes_namespace            = kubernetes_namespace.monitoring.metadata[0].name
  environment                     = var.environment
}

resource "google_project_iam_member" "stackdriver_exporter" {
  count = var.use_aws ? 0 : 1

  project = var.gcp.project
  role    = "roles/monitoring.viewer"
  member  = "serviceAccount:${module.account_mapping.gcp_service_account_email}"
}

resource "aws_iam_role_policy" "cloudwatch_exporter" {
  count = var.use_aws ? 1 : 0

  name = "${var.environment}-prometheus-cloudwatch-exporter"
  role = module.account_mapping.aws_iam_role.name

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action   = ["cloudwatch:ListMetrics", "cloudwatch:GetMetricStatistics"]
        Effect   = "Allow"
        Resource = "*"
      }
    ]
  })
}
