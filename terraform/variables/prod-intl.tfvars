environment     = "prod-intl"
state_bucket    = "prod-intl-us-west1-prio-terraform"
gcp_region      = "us-west1"
gcp_project     = "prio-intl-prod"
localities      = ["mn", "mx-coa", "mx-jal", "mx-pue", "mx-yuc"]
aws_region      = "us-west-2"
manifest_domain = "isrg-prio.org"
ingestors = {
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      mn = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-coa = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-jal = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-pue = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-yuc = {
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
      mn = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-coa = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-jal = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-pue = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-yuc = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 3
  # note that this size only applies to the spot node group and we will always
  # have 3 nodes in the on demand node group
  min_node_count    = 0
  max_node_count    = 6
  gcp_machine_type  = "e2-standard-2"
  aws_machine_types = ["t3.large"]
}
is_first                 = false
use_aws                  = true
pure_gcp                 = true
facilitator_version      = "0.6.22"
workflow_manager_version = "0.6.22"
key_rotator_version      = "0.6.22"
victorops_routing_key    = "prio-prod-intl"

default_aggregation_period       = "8h"
default_aggregation_grace_period = "4h"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-enpa-g-prod-manifests"
default_portal_server_manifest_base_url        = "manifest.global.enpa-pha.io"

prometheus_helm_chart_version          = "15.0.2"
grafana_helm_chart_version             = "6.20.5"
cloudwatch_exporter_helm_chart_version = "1.12.0"
