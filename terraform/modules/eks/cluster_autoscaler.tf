# Configures a cluster autoscaler for EKS, according to AWS documentation
# https://docs.aws.amazon.com/eks/latest/userguide/autoscaling.html#cluster-autoscaler
# https://raw.githubusercontent.com/kubernetes/autoscaler/master/cluster-autoscaler/cloudprovider/aws/examples/cluster-autoscaler-autodiscover.yaml

data "kubernetes_namespace" "kube_system" {
  metadata {
    name = "kube-system"
  }
}

# Create a Kubernetes service account mapped to an AWS IAM role that is capable
# of autoscaling a node group
module "account_mapping" {
  source            = "../account_mapping"
  aws_iam_role_name = "${var.environment}-cluster-autoscaler"
  eks_oidc_provider = {
    url = local.oidc_provider_url,
    arn = aws_iam_openid_connect_provider.oidc.arn,
  }
  kubernetes_service_account_name = "cluster-autoscaler"
  kubernetes_service_account_labels = {
    k8s-addon = "cluster-autoscaler.addons.k8s.io"
    k8s-app   = "cluster-autoscaler"
  }
  kubernetes_namespace = data.kubernetes_namespace.kube_system.metadata[0].name
  environment          = var.environment
}

resource "aws_iam_role_policy" "cluster_autoscaler" {
  name = "cluster-autoscaler"
  role = module.account_mapping.aws_iam_role.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action = [
          "autoscaling:DescribeAutoScalingGroups",
          "autoscaling:DescribeAutoScalingInstances",
          "autoscaling:DescribeLaunchConfigurations",
          "autoscaling:DescribeTags",
          "autoscaling:SetDesiredCapacity",
          "autoscaling:TerminateInstanceInAutoScalingGroup",
          "ec2:DescribeLaunchTemplateVersions",
        ]
        Resource = "*"
        Effect   = "Allow"
      }
    ]
  })
}

resource "kubernetes_cluster_role" "cluster_autoscaler" {
  metadata {
    name = "cluster-autoscaler"
    labels = {
      k8s-addon = "cluster-autoscaler.addons.k8s.io"
      k8s-app   = "cluster-autoscaler"
    }
  }

  rule {
    api_groups = [""]
    resources  = ["events", "endpoints"]
    verbs      = ["create", "patch"]
  }

  rule {
    api_groups = [""]
    resources  = ["pods/eviction"]
    verbs      = ["create"]
  }

  rule {
    api_groups = [""]
    resources  = ["pods/status"]
    verbs      = ["update"]
  }

  rule {
    api_groups     = [""]
    resources      = ["endpoints"]
    resource_names = ["cluster-autoscaler"]
    verbs          = ["get", "update"]
  }

  rule {
    api_groups = [""]
    resources  = ["nodes"]
    verbs      = ["watch", "list", "get", "update"]
  }

  rule {
    api_groups = [""]
    resources = [
      "namespaces",
      "pods",
      "services",
      "replicationcontrollers",
      "persistentvolumeclaims",
      "persistentvolumes",
    ]
    verbs = ["watch", "list", "get"]
  }

  rule {
    api_groups = ["extensions"]
    resources  = ["replicasets", "daemonsets"]
    verbs      = ["watch", "list", "get"]
  }

  rule {
    api_groups = ["policy"]
    resources  = ["poddisruptionbudgets"]
    verbs      = ["watch", "list"]
  }

  rule {
    api_groups = ["apps"]
    resources  = ["statefulsets", "replicasets", "daemonsets"]
    verbs      = ["watch", "list", "get"]
  }

  rule {
    api_groups = ["storage.k8s.io"]
    resources  = ["storageclasses", "csinodes", "csidrivers", "csistoragecapacities"]
    verbs      = ["watch", "list", "get"]
  }

  rule {
    api_groups = ["batch", "extensions"]
    resources  = ["jobs"]
    verbs      = ["get", "list", "watch", "patch"]
  }

  rule {
    api_groups = ["coordination.k8s.io"]
    resources  = ["leases"]
    verbs      = ["create"]
  }

  rule {
    api_groups     = ["coordination.k8s.io"]
    resource_names = ["cluster-autoscaler"]
    resources      = ["leases"]
    verbs          = ["get", "update"]
  }
}

resource "kubernetes_cluster_role_binding" "cluster_autoscaler" {
  metadata {
    name = "cluster-autoscaler"
    labels = {
      k8s-addon = "cluster-autoscaler.addons.k8s.io"
      k8s-app   = "cluster-autoscaler"
    }
  }

  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = kubernetes_cluster_role.cluster_autoscaler.metadata[0].name
  }

  subject {
    kind      = "ServiceAccount"
    name      = module.account_mapping.kubernetes_service_account_name
    namespace = data.kubernetes_namespace.kube_system.metadata[0].name
  }
}

resource "kubernetes_role" "cluster_autoscaler" {
  metadata {
    name      = "cluster-autoscaler"
    namespace = data.kubernetes_namespace.kube_system.metadata[0].name
    labels = {
      k8s-addon = "cluster-autoscaler.addons.k8s.io"
      k8s-app   = "cluster-autoscaler"
    }
  }

  rule {
    api_groups = [""]
    resources  = ["configmaps"]
    verbs      = ["create", "list", "watch"]
  }

  rule {
    api_groups     = [""]
    resources      = ["configmaps"]
    resource_names = ["cluster-autoscaler-status", "cluster-autoscaler-priority-expander"]
    verbs          = ["delete", "get", "update", "watch"]
  }
}

resource "kubernetes_role_binding" "cluster_autoscaler" {
  metadata {
    name      = "cluster-autoscaler"
    namespace = data.kubernetes_namespace.kube_system.metadata[0].name
    labels = {
      k8s-addon = "cluster-autoscaler.addons.k8s.io"
      k8s-app   = "cluster-autoscaler"
    }
  }

  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "Role"
    name      = kubernetes_role.cluster_autoscaler.metadata[0].name
  }

  subject {
    kind      = "ServiceAccount"
    name      = module.account_mapping.kubernetes_service_account_name
    namespace = data.kubernetes_namespace.kube_system.metadata[0].name
  }
}

resource "kubernetes_deployment" "cluster_autoscaler" {
  metadata {
    name      = "cluster-autoscaler"
    namespace = data.kubernetes_namespace.kube_system.metadata[0].name
    labels = {
      app = "cluster-autoscaler"
    }
  }

  spec {
    replicas = 1
    selector {
      match_labels = {
        app = "cluster-autoscaler"
      }
    }

    template {
      metadata {
        labels = {
          app = "cluster-autoscaler"
        }
        annotations = {
          "prometheus.io/scrape"                           = "true"
          "prometheus.io/port"                             = "8085"
          "cluster-autoscaler.kubernetes.io/safe-to-evict" = "false"
        }
      }

      spec {
        priority_class_name = "system-cluster-critical"
        security_context {
          run_as_non_root = true
          run_as_user     = 65534
          fs_group        = 65534
        }
        service_account_name = module.account_mapping.kubernetes_service_account_name

        container {
          # cluster-autoscaler image version should correspond to Kubernetes cluster version
          # https://github.com/kubernetes/autoscaler/releases/tag/cluster-autoscaler-1.20.1
          image = "k8s.gcr.io/autoscaling/cluster-autoscaler:v1.20.1"
          name  = "cluster-autoscaler"

          resources {
            limits = {
              cpu    = "100m"
              memory = "600Mi"
            }
            requests = {
              cpu    = "100m"
              memory = "600Mi"
            }
          }

          command = [
            "./cluster-autoscaler",
            "--v=4",
            "--stderrthreshold=info",
            "--cloud-provider=aws",
            "--skip-nodes-with-local-storage=false",
            "--expander=least-waste",
            "--node-group-auto-discovery=asg:tag=k8s.io/cluster-autoscaler/enabled,k8s.io/cluster-autoscaler/${aws_eks_cluster.cluster.name}",
            "--balance-similar-node-groups",
            "--skip-nodes-with-system-pods=false",
          ]

          volume_mount {
            name       = "ssl-certs"
            mount_path = "/etc/ssl/certs/ca-bundle.crt"
            read_only  = true
          }

          image_pull_policy = "Always"
        }
        volume {
          name = "ssl-certs"
          host_path {
            path = "/etc/ssl/certs/ca-bundle.crt"
          }
        }
      }
    }
  }
}
