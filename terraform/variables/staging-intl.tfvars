environment     = "staging-intl"
state_bucket    = "staging-intl-us-west1-prio-terraform"
gcp_region      = "us-west1"
gcp_project     = "prio-intl-staging"
localities      = ["na-na"]
aws_region      = "us-west-2"
manifest_domain = "isrg-prio.org"
ingestors = {
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      na-na = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
  apple = {
    manifest_base_url = "exposure-notification.apple.com/manifest"
    localities = {
      na-na = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count  = 1
  min_node_count      = 0
  max_node_count      = 3
  gcp_machine_type    = "e2-standard-2"
  aws_machine_types   = ["t3.large"]
  eks_cluster_version = "1.23"
  # https://docs.aws.amazon.com/eks/latest/userguide/managing-vpc-cni.html
  eks_vpc_cni_addon_version = "v1.12.2-eksbuild.1"
  # https://docs.aws.amazon.com/eks/latest/userguide/managing-ebs-csi.html#updating-ebs-csi-eks-add-on
  eks_ebs_csi_addon_version = "v1.16.0-eksbuild.1"
  # https://github.com/kubernetes/autoscaler/releases/tag/cluster-autoscaler-1.23.0
  eks_cluster_autoscaler_version = "registry.k8s.io/autoscaling/cluster-autoscaler:v1.23.0"
}
is_first                 = false
use_aws                  = true
pure_gcp                 = true
workflow_manager_version = "0.6.88"
facilitator_version      = "0.6.88"
key_rotator_version      = "0.6.88"
victorops_routing_key    = "prio-staging"

default_aggregation_period       = "30m"
default_aggregation_grace_period = "10m"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-staging-server-manifests"
default_portal_server_manifest_base_url        = "manifest.int.enpa-pha.io"

prometheus_helm_chart_version           = "15.9.0"
grafana_helm_chart_version              = "6.20.5"
cloudwatch_exporter_helm_chart_version  = "0.22.0"
stackdriver_exporter_helm_chart_version = "2.2.0"

allowed_aws_account_ids = ["024759592502"]
