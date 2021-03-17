environment     = "staging-pha"
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
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"
      }
      gondor = {
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"
      }
      asgard = {
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"
      }
      gondor = {
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"
      }
      asgard = {
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
        portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 2
  min_node_count     = 1
  max_node_count     = 3
  machine_type       = "e2-standard-2"
}
test_peer_environment = {
  env_with_ingestor            = "staging-facil"
  env_without_ingestor         = "staging-pha"
  localities_with_sample_maker = ["narnia", "gondor"]
}
is_first                 = true
use_aws                  = true
workflow_manager_version = "0.6.10"
facilitator_version      = "0.6.10"
pushgateway              = "prometheus-pushgateway.monitoring:9091"
victorops_routing_key    = "prio-staging"
aggregation_period       = "30m"
