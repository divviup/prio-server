environment     = "staging-facil"
gcp_region      = "us-west1"
gcp_project     = "prio-staging-300104"
localities      = ["narnia", "gondor", "asgard"]
aws_region      = "us-west-1"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  ingestor-1 = {
    manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      gondor = {
        min_intake_worker_count    = 2
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      asgard = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      gondor = {
        min_intake_worker_count    = 2
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      asgard = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 2
  min_node_count     = 1
  max_node_count     = 3
  gcp_machine_type   = "e2-standard-2"
  aws_machine_types  = []
}
test_peer_environment = {
  env_with_ingestor            = "staging-facil"
  env_without_ingestor         = "staging-pha"
  localities_with_sample_maker = ["narnia", "gondor"]
}
is_first                 = false
use_aws                  = false
workflow_manager_version = "0.6.19"
facilitator_version      = "0.6.19"
key_rotator_version      = "0.6.19"
victorops_routing_key    = "prio-staging"

intake_max_age             = "75m"
default_aggregation_period = "30m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-pha-manifests"
default_portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-facil-manifests/portal-server"

key_rotator_schedule           = "*/10 * * * *" // once per ten minutes
enable_key_rotation_localities = ["*"]
batch_signing_key_rotation_policy = {
  create_min_age   = "6480h" // 6480 hours = 9 months (w/ 30-day months, 24-hour days)
  primary_min_age  = "6h"    // 6 hours
  delete_min_age   = "9360h" // 9360 hours = 13 months (w/ 30-day months, 24-hour days)
  delete_min_count = 2
}
