environment     = "timg-dev-facil"
gcp_region      = "us-west1"
gcp_project     = "timg-prio-dev"
localities      = ["narnia"]
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
        aggregate_thread_count                 = 2
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-dev-pha-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-timg-dev-facil-manifests/portal-server"
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-timg-dev-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        aggregate_thread_count                 = 4
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-dev-pha-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-timg-dev-facil-manifests/portal-server"
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 1
  min_node_count     = 1
  max_node_count     = 2
  machine_type       = "e2-standard-2"
}
test_peer_environment = {
  env_with_ingestor            = "timg-dev-facil"
  env_without_ingestor         = "timg-dev-pha"
  localities_with_sample_maker = ["narnia"]
}
is_first                 = false
use_aws                  = false
container_registry       = "us.gcr.io/timg-prio-dev"
pushgateway              = "prometheus-pushgateway.monitoring:9091"
aggregation_period       = "30m"
aggregation_grace_period = "10m"
