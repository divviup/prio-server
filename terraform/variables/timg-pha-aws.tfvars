environment     = "timg-pha-aws"
gcp_region      = "us-west1"
gcp_project     = "timg-prio-aws-dev"
localities      = ["gondor"]
aws_region      = "eu-central-1"
aws_profile     = "bringup"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  ingestor-1 = {
    manifest_base_url = "storage.googleapis.com/prio-timg-facil-aws-manifests/singleton-ingestor"
    localities = {
      gondor = {
        intake_worker_count                    = 1
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-facil-aws-manifests"
        portal_server_manifest_base_url        = "isrg-prio-timg-pha-aws-manifests.s3.amazonaws.com/portal-server"
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-timg-facil-aws-manifests/singleton-ingestor"
    localities = {
      gondor = {
        intake_worker_count                    = 1
        aggregate_worker_count                 = 1
        peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-timg-facil-aws-manifests"
        portal_server_manifest_base_url        = "isrg-prio-timg-pha-aws-manifests.s3.amazonaws.com/portal-server"
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 1
  min_node_count     = 1
  max_node_count     = 2
  gcp_machine_type   = "e2-standard-2"
  aws_machine_types  = ["t3.large"]
}
test_peer_environment = {
  env_with_ingestor            = "timg-facil-aws"
  env_without_ingestor         = "timg-pha-aws"
  localities_with_sample_maker = ["gondor"]
}
is_first                 = true
use_aws                  = true
workflow_manager_version = "0.6.13"
facilitator_version      = "0.6.13"
#pushgateway              = "prometheus-pushgateway.monitoring:9091"
aggregation_period       = "30m"
aggregation_grace_period = "10m"
