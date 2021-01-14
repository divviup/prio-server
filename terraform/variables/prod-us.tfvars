environment     = "prod-us"
gcp_region      = "us-west1"
gcp_project     = "prio-prod-us"
machine_type    = "e2-standard-8"
localities      = ["aq-aq", "ta-ta", "us-ct", "us-md"]
aws_region      = "us-west-1"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  apple = {
    manifest_base_url = "exposure-notification.apple.com/manifest"
    localities = {
      aq-aq = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
      ta-ta = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
      us-ct = {
        intake_worker_count    = 5
        aggregate_worker_count = 3
      }
      us-md = {
        intake_worker_count    = 5
        aggregate_worker_count = 3
      }
    }
  }
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      aq-aq = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
      ta-ta = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
      us-ct = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-md = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
    }
  }
}
peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
portal_server_manifest_base_url        = "manifest.enpa-pha.io"
is_first                               = false
use_aws                                = false
aggregation_period                     = "8h"
aggregation_grace_period               = "4h"
workflow_manager_version               = "0.6.0"
facilitator_version                    = "0.6.0"
pushgateway                            = "prometheus-pushgateway.monitoring:9091"

cluster_settings = {
  initial_node_count : 4
  min_node_count : 4
  max_node_count : 5
}