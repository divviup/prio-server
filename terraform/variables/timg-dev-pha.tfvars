environment     = "timg-dev-pha"
gcp_region      = "us-west1"
gcp_project     = "timg-prio-dev"
localities      = ["narnia", "gondor"]
aws_region      = "us-west-1"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  ingestor-1 = {
    manifest_base_url = "storage.googleapis.com/prio-timg-dev-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-dev-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-timg-dev-pha-manifests/portal-server"
      }
      gondor = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-dev-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-timg-dev-pha-manifests/portal-server"
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-timg-dev-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-dev-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-timg-dev-pha-manifests/portal-server"
      }
      gondor = {
        intake_worker_count                    = 3
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-dev-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-timg-dev-pha-manifests/portal-server"
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 1
  min_node_count     = 1
  max_node_count     = 2
  gcp_machine_type   = "e2-standard-2"
  aws_machine_types  = ["t3a.small"]
}
test_peer_environment = {
  env_with_ingestor            = "timg-dev-facil"
  env_without_ingestor         = "timg-dev-pha"
  localities_with_sample_maker = ["narnia", "gondor"]
}
is_first                 = true
use_aws                  = true
container_registry       = "us.gcr.io/timg-prio-dev"
facilitator_version      = "retries-and-log-2"
workflow_manager_version = "retries-and-log-1"
pushgateway              = "prometheus-pushgateway.monitoring:9091"
aggregation_period       = "30m"
aggregation_grace_period = "10m"
