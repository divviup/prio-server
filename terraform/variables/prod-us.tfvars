environment     = "prod-us"
gcp_region      = "us-west1"
gcp_project     = "prio-prod-us"
localities      = ["ta-ta", "us-ct", "us-md", "us-va", "us-wa", "us-ca", "us-ut", "us-wi", "us-ma", "us-nm"]
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
      # ta-ta is reserved for integration testing with NCI and MITRE's staging
      # instances
      ta-ta = {
        intake_worker_count                    = 1
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "test-en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
      }
      us-ct = {
        intake_worker_count                    = 5
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-md = {
        intake_worker_count                    = 5
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-va = {
        intake_worker_count                    = 5
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wa = {
        intake_worker_count                    = 5
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ca = {
        intake_worker_count                    = 25
        aggregate_worker_count                 = 7
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ut = {
        intake_worker_count                    = 8
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wi = {
        intake_worker_count                    = 12
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ma = {
        intake_worker_count                    = 12
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-nm = {
        intake_worker_count                    = 5
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
    }
  }
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      # ta-ta is reserved for integration testing with NCI and MITRE's staging
      # instances
      ta-ta = {
        intake_worker_count                    = 1
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "test-en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
      }
      us-ct = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-md = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-va = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wa = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ca = {
        intake_worker_count                    = 15
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ut = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wi = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ma = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-nm = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 3
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 4
  min_node_count     = 1
  max_node_count     = 5
  machine_type       = "e2-standard-8"
}
is_first                                  = false
use_aws                                   = false
aggregation_period                        = "8h"
aggregation_grace_period                  = "4h"
workflow_manager_version                  = "0.6.10"
facilitator_version                       = "0.6.10"
pushgateway                               = "prometheus-pushgateway.monitoring:9091"
prometheus_server_persistent_disk_size_gb = 1000
victorops_routing_key                     = "prio-prod-us"
