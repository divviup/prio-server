environment     = "prod-intl"
state_bucket    = "prod-intl-us-west1-prio-terraform"
gcp_region      = "us-west1"
gcp_project     = "prio-intl-prod"
localities      = ["mn", "mx-coa", "mx-jal", "mx-pue", "mx-yuc", "rw"]
aws_region      = "us-west-2"
manifest_domain = "isrg-prio.org"
ingestors = {
  g-enpa = {
    manifest_base_url = "storage.googleapis.com/prio-manifests"
    localities = {
      mn = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-coa = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-jal = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-pue = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-yuc = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      rw = {
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
      mn = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-coa = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-jal = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-pue = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      mx-yuc = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
      rw = {
        min_intake_worker_count    = 1
        max_intake_worker_count    = 3
        min_aggregate_worker_count = 1
        max_aggregate_worker_count = 3
      }
    }
  }
}
cluster_settings = {
  initial_node_count = 3
  # note that this size only applies to the spot node group and we will always
  # have 3 nodes in the on demand node group
  min_node_count      = 0
  max_node_count      = 6
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
workflow_manager_version = "0.6.99"
facilitator_version      = "0.6.99"
key_rotator_version      = "0.6.99"
victorops_routing_key    = "prio-prod-intl"

default_aggregation_period       = "8h"
default_aggregation_grace_period = "4h"

default_peer_share_processor_manifest_base_url = "storage.googleapis.com/prio-enpa-g-prod-manifests"
default_portal_server_manifest_base_url        = "manifest.global.enpa-pha.io"

enable_key_rotation_localities = ["*"]

# The packet encryption key rotation policy is the default, except for the
# delete_min_count. See https://github.com/divviup/prio-server/issues/2248 for
# details.
packet_encryption_key_rotation_policy = {
  create_min_age   = "6480h" # default value
  primary_min_age  = "23h"   # default value
  delete_min_age   = "9360h" # default value
  delete_min_count = 3       # allow 3 concurrent keys so we don't delete the in-use version 0 key
}

prometheus_helm_chart_version           = "15.9.0"
grafana_helm_chart_version              = "6.20.5"
cloudwatch_exporter_helm_chart_version  = "0.22.0"
stackdriver_exporter_helm_chart_version = "2.2.0"

allowed_aws_account_ids = ["718214359651"]
