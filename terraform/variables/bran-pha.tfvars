environment     = "bran-pha"
state_bucket    = "bran-pha-us-west1-prio-terraform"
gcp_region      = "us-west1"
gcp_project     = "prio-bran-dev"
localities      = ["narnia", "gondor", "asgard", "eorzea"]
aws_region      = "us-west-1"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  ingestor-1 = {
    manifest_base_url = "storage.googleapis.com/prio-bran-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
      gondor = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
      asgard = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
      eorzea = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-bran-facil-manifests/singleton-ingestor"
    localities = {
      narnia = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
      gondor = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
      asgard = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
      eorzea = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 5
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 5
      }
    }
  }
}
test_peer_environment = {
  env_with_ingestor            = "bran-facil"
  env_without_ingestor         = "bran-pha"
  localities_with_sample_maker = ["narnia", "gondor"]
}
is_first                 = true
use_aws                  = true
pure_gcp                 = true
container_registry       = "811485267624.dkr.ecr.us-east-2.amazonaws.com/letsencrypt"
workflow_manager_version = "latest"
facilitator_version      = "latest"
key_rotator_version      = "latest"
victorops_routing_key    = "prio-brandon-dev"

default_aggregation_period = "30m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-bran-facil-manifests"
default_portal_server_manifest_base_url        = "isrg-prio-bran-pha-manifest.s3.amazonaws.com/portal-server"

key_rotator_schedule                      = "*/10 * * * *" // once per ten minutes
enable_key_rotation_localities            = ["*"]
single_object_validation_batch_localities = ["narnia", "gondor"]

prometheus_helm_chart_version           = "15.0.2"
grafana_helm_chart_version              = "6.20.5"
cloudwatch_exporter_helm_chart_version  = "0.17.2"
stackdriver_exporter_helm_chart_version = "1.12.0"
