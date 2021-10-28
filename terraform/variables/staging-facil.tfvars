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
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
      gondor = {
        intake_worker_count    = 2
        aggregate_worker_count = 1
      }
      asgard = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        intake_worker_count    = 2
        aggregate_worker_count = 1
      }
      gondor = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
      }
      asgard = {
        intake_worker_count    = 1
        aggregate_worker_count = 1
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
workflow_manager_version = "0.6.17"
facilitator_version      = "0.6.17"
victorops_routing_key    = "prio-staging"

intake_max_age = "30m"
default_aggregation_period = "30m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-pha-manifests"
default_portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-facil-manifests/portal-server"
