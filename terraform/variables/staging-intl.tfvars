environment     = "staging-intl"
state_bucket    = "staging-intl-us-west1-prio-terraform"
gcp_region      = "us-west1"
gcp_project     = "prio-intl-staging"
localities      = ["na-na"]
aws_region      = "us-west-2"
manifest_domain = "isrg-prio.org"
ingestors = {
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      na-na = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
  apple = {
    manifest_base_url = "exposure-notification.apple.com/manifest"
    localities = {
      na-na = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 1
  min_node_count     = 0
  max_node_count     = 3
  gcp_machine_type   = "e2-standard-2"
  aws_machine_types  = ["t3.large"]
}
is_first                 = false
use_aws                  = true
pure_gcp                 = true
workflow_manager_version = "0.6.22"
facilitator_version      = "0.6.22"
key_rotator_version      = "0.6.22"
victorops_routing_key    = "prio-staging"

default_aggregation_period       = "30m"
default_aggregation_grace_period = "10m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-server-manifests"
default_portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"

prometheus_helm_chart_version           = "15.0.2"
grafana_helm_chart_version              = "6.20.5"
cloudwatch_exporter_helm_chart_version  = "0.17.2"
stackdriver_exporter_helm_chart_version = "1.12.0"

# Uncomment and fill in this variable to require that this file always be used
# with the same AWS account.
# allowed_aws_account_ids = ["123456789012"]
