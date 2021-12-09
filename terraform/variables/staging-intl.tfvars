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
workflow_manager_version = "0.6.19"
facilitator_version      = "0.6.19"
key_rotator_version      = "0.6.19"
victorops_routing_key    = "prio-staging"

default_aggregation_period       = "30m"
default_aggregation_grace_period = "10m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-server-manifests"
default_portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
