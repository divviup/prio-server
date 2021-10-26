environment     = "prod-us"
gcp_region      = "us-west1"
gcp_project     = "prio-prod-us"
localities      = ["ta-ta", "us-ct", "us-md", "us-va", "us-wa", "us-ca", "us-ut", "us-wi", "us-ma", "us-nm", "us-hi", "us-co", "us-dc", "us-la", "us-mo"]
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
        aggregation_period                     = "30m"
        aggregation_grace_period               = "30m"
      }
      us-ct = {
        intake_worker_count    = 8
        aggregate_worker_count = 3
      }
      us-md = {
        intake_worker_count    = 8
        aggregate_worker_count = 3
      }
      us-va = {
        intake_worker_count    = 8
        aggregate_worker_count = 3
      }
      us-wa = {
        intake_worker_count    = 10
        aggregate_worker_count = 3
      }
      us-ca = {
        intake_worker_count    = 25
        aggregate_worker_count = 7
      }
      us-ut = {
        intake_worker_count    = 10
        aggregate_worker_count = 3
      }
      us-wi = {
        intake_worker_count    = 12
        aggregate_worker_count = 3
      }
      us-ma = {
        intake_worker_count    = 12
        aggregate_worker_count = 3
      }
      us-nm = {
        intake_worker_count    = 5
        aggregate_worker_count = 3
      }
      us-hi = {
        intake_worker_count    = 5
        aggregate_worker_count = 3
      }
      us-co = {
        intake_worker_count    = 8
        aggregate_worker_count = 3
      }
      us-dc = {
        intake_worker_count    = 5
        aggregate_worker_count = 3
      }
      us-la = {
        intake_worker_count    = 5
        aggregate_worker_count = 3
      }
      us-mo = {
        intake_worker_count    = 5
        aggregate_worker_count = 3
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
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-md = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-va = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-wa = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-ca = {
        intake_worker_count    = 15
        aggregate_worker_count = 3
      }
      us-ut = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-wi = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-ma = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-nm = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-hi = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-co = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-dc = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-la = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
      us-mo = {
        intake_worker_count    = 3
        aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 4
  min_node_count     = 1
  max_node_count     = 5
  gcp_machine_type   = "e2-standard-8"
  aws_machine_types  = []
}
is_first                                  = false
use_aws                                   = false
default_aggregation_period                = "8h"
default_aggregation_grace_period          = "4h"
workflow_manager_version                  = "0.6.17"
facilitator_version                       = "0.6.17"
prometheus_server_persistent_disk_size_gb = 1000
victorops_routing_key                     = "prio-prod-us"

default_peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
default_portal_server_manifest_base_url        = "manifest.enpa-pha.io"
