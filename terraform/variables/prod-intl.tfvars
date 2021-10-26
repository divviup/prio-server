environment     = "prod-intl"
gcp_region      = "us-west1"
gcp_project     = "prio-intl-prod"
localities      = ["mn-mn"]
aws_region      = "us-west-2"
manifest_domain = "isrg-prio.org"
ingestors = {
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      mn-mn = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
    }
  }
  apple = {
    manifest_base_url = "exposure-notification.apple.com/manifest"
    localities = {
      mn-mn = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 3
  min_node_count     = 3
  max_node_count     = 3
  gcp_machine_type   = "e2-standard-2"
  aws_machine_types  = ["t3.large"]
}
is_first                 = false
use_aws                  = true
pure_gcp                 = true
facilitator_version      = "0.6.17"
workflow_manager_version = "0.6.17"
victorops_routing_key    = "prio-prod-intl"

default_aggregation_period       = "8h"
default_aggregation_grace_period = "4h"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-enpa-g-prod-manifests"
default_portal_server_manifest_base_url        = "manifest.global.enpa-pha.io"
