environment     = "staging-intl"
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
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
    }
  }
  apple = {
    manifest_base_url = "exposure-notification.apple.com/manifest"
    localities = {
      na-na = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 2
  min_node_count     = 1
  max_node_count     = 3
  gcp_machine_type   = "e2-standard-2"
  aws_machine_types  = ["t3.large"]
}
is_first                 = false
use_aws                  = true
pure_gcp                 = true
facilitator_version      = "0.6.16"
workflow_manager_version = "0.6.16"
pushgateway              = "prometheus-pushgateway.monitoring:9091"
victorops_routing_key    = "prio-staging"

default_aggregation_period       = "30m"
default_aggregation_grace_period = "10m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-server-manifests"
default_portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
