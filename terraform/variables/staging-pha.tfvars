environment     = "staging-pha"
gcp_region      = "us-west1"
gcp_project     = "prio-staging-300104"
machine_type    = "e2-standard-8"
localities      = ["narnia", "gondor", "asgard"]
aws_region      = "us-west-1"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  ingestor-1 = {
    manifest_base_url = "storage.googleapis.com/prio-staging-pha-manifests/ingestor-1"
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
    manifest_base_url = "storage.googleapis.com/prio-staging-pha-manifests/ingestor-2"
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
peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-facil-manifests"
portal_server_manifest_base_url        = "storage.googleapis.com/prio-staging-pha-manifests/portal-server"
test_peer_environment = {
  env_with_ingestor    = "staging-facil"
  env_without_ingestor = "staging-pha"
}
is_first                 = true
use_aws                  = true
workflow_manager_version = "0.6.0"
facilitator_version      = "0.6.0"
pushgateway              = "prometheus-pushgateway.monitoring:9091"
