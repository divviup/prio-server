environment     = "staging-pha"
state_bucket    = "staging-pha-us-west1-prio-terraform"
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
        min_intake_worker_count    = 1
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
        min_intake_worker_count    = 1
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
}
is_first                 = true
use_aws                  = true
workflow_manager_version = "0.6.18"
facilitator_version      = "0.6.18"
victorops_routing_key    = "prio-staging"

intake_max_age             = "75m"
default_aggregation_period = "30m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
default_portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"

allowed_aws_account_ids = ["338276578713"]
