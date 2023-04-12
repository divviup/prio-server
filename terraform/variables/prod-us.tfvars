environment     = "prod-us"
state_bucket    = "prod-us-us-west1-prio-terraform"
gcp_region      = "us-west1"
gcp_project     = "prio-prod-us"
localities      = ["ta-ta", "us-ct", "us-md", "us-va", "us-wa", "us-ca", "us-ut", "us-wi", "us-ma", "us-nm", "us-hi", "us-co", "us-dc", "us-la", "us-mo", "us-ak", "us-nv"]
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
        min_intake_worker_count                = 1
        max_intake_worker_count                = 3
        min_aggregate_worker_count             = 1
        max_aggregate_worker_count             = 3
        peer_share_processor_manifest_base_url = "test-en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
        aggregation_period                     = "30m"
        aggregation_grace_period               = "30m"
      }
      us-ct = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-md = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-va = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-wa = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-ca = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 20
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 9
      }
      us-ut = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-wi = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-ma = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-nm = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-hi = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-co = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-dc = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-la = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-mo = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-ak = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-nv = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 12
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      # ta-ta is reserved for integration testing with NCI and MITRE's staging
      # instances
      ta-ta = {
        min_intake_worker_count                = 1
        max_intake_worker_count                = 3
        min_aggregate_worker_count             = 1
        max_aggregate_worker_count             = 3
        peer_share_processor_manifest_base_url = "test-en-analytics.cancer.gov"
        portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"
      }
      us-ct = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-md = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-va = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-wa = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-ca = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 10
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-ut = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-wi = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-ma = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-nm = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-hi = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-co = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-dc = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-la = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-mo = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-ak = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      us-nv = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 6
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 4
  min_node_count     = 0
  max_node_count     = 5
  gcp_machine_type   = "e2-standard-8"
  aws_machine_types  = []
}
is_first                                  = false
use_aws                                   = false
default_aggregation_period                = "8h"
default_aggregation_grace_period          = "4h"
workflow_manager_version                  = "0.6.89"
facilitator_version                       = "0.6.89"
key_rotator_version                       = "0.6.89"
prometheus_server_persistent_disk_size_gb = 1000
victorops_routing_key                     = "prio-prod-us"

default_peer_share_processor_manifest_base_url = "en-analytics.cancer.gov"
default_portal_server_manifest_base_url        = "manifest.enpa-pha.io"

enable_key_rotation_localities = ["*"]

prometheus_helm_chart_version           = "15.9.0"
grafana_helm_chart_version              = "6.20.5"
cloudwatch_exporter_helm_chart_version  = "0.22.0"
stackdriver_exporter_helm_chart_version = "2.2.0"

allowed_aws_account_ids = ["338276578713"]
