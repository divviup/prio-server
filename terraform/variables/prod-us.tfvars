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
        aggregate_thread_count                 = 1
        peer_share_processor_manifest_base_url = "test-en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
      }
      us-ct = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-md = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-va = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wa = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ca = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ut = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wi = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ma = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-nm = {
        aggregate_thread_count                 = 4
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
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "test-en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
      }
      us-ct = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-md = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-va = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wa = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ca = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ut = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-wi = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-ma = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.enpa-pha.io"
      }
      us-nm = {
        aggregate_thread_count                 = 4
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
