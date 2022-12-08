environment     = "stg-us-pha"
state_bucket    = "stg-us-pha-us-west1-prio-terraform"
gcp_region      = "us-west1"
gcp_project     = "prio-staging-us"
localities      = ["narnia", "gondor", "asgard"]
aws_region      = "us-west-1"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  ingestor-1 = {
    manifest_base_url = "storage.googleapis.com/prio-stg-us-facil-manifests/singleton-ingestor"
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
        pure_gcp                   = true
      }
      asgard = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
        pure_gcp                   = true
      }
    }
  }
  ingestor-2 = {
    manifest_base_url = "storage.googleapis.com/prio-stg-us-facil-manifests/singleton-ingestor"
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
        pure_gcp                   = true
      }
      asgard = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
        pure_gcp                   = true
      }
    }
  }
}
cluster_settings = {
  initial_node_count  = 2
  min_node_count      = 0
  max_node_count      = 3
  gcp_machine_type    = "e2-standard-2"
  aws_machine_types   = ["t3.medium"]
  eks_cluster_version = "1.22"
  # https://docs.aws.amazon.com/eks/latest/userguide/managing-vpc-cni.html
  eks_vpc_cni_addon_version = "v1.11.4-eksbuild.1"
  # https://docs.aws.amazon.com/eks/latest/userguide/managing-ebs-csi.html#updating-ebs-csi-eks-add-on
  eks_ebs_csi_addon_version = "v1.13.0-eksbuild.2"
  # https://github.com/kubernetes/autoscaler/releases/tag/cluster-autoscaler-1.22.1
  eks_cluster_autoscaler_version = "k8s.gcr.io/autoscaling/cluster-autoscaler:v1.22.1"
}
test_peer_environment = {
  env_with_ingestor    = "stg-us-facil"
  env_without_ingestor = "stg-us-pha"
}
is_first                 = true
use_aws                  = true
workflow_manager_version = "0.6.74"
facilitator_version      = "0.6.74"
key_rotator_version      = "0.6.74"
victorops_routing_key    = "prio-staging"

intake_max_age             = "75m"
default_aggregation_period = "30m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-stg-us-facil-manifests"
default_portal_server_manifest_base_url        = "isrg-prio-stg-us-pha-manifest.s3.amazonaws.com/portal-server"

key_rotator_schedule           = "*/10 * * * *" // once per ten minutes
enable_key_rotation_localities = ["*"]
batch_signing_key_rotation_policy = {
  create_min_age   = "6480h" // 6480 hours = 9 months (w/ 30-day months, 24-hour days)
  primary_min_age  = "6h"    // 6 hours
  delete_min_age   = "9360h" // 9360 hours = 13 months (w/ 30-day months, 24-hour days)
  delete_min_count = 2
}

single_object_validation_batch_localities = ["gondor", "narnia"]

prometheus_helm_chart_version           = "15.9.0"
grafana_helm_chart_version              = "6.20.5"
cloudwatch_exporter_helm_chart_version  = "0.17.2"
stackdriver_exporter_helm_chart_version = "2.2.0"

allowed_aws_account_ids = ["183375168988"]
