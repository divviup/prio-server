variable "environment" {
  type = string
}

variable "resource_prefix" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "gcp_project" {
  type = string
}

variable "cluster_settings" {
  type = object({
    initial_node_count = number
    min_node_count     = number
    max_node_count     = number
    gcp_machine_type   = string
    aws_machine_types  = list(string)
  })
}

# Activate some services which the deployment will require.
resource "google_project_service" "compute" {
  service = "compute.googleapis.com"
}

resource "google_project_service" "container" {
  service = "container.googleapis.com"
}

resource "google_project_service" "kms" {
  service = "cloudkms.googleapis.com"
}

resource "google_container_cluster" "cluster" {
  # The google provider seems to have not been updated to consider VPC-native
  # clusters as GA, even though they are GA and in fact are now the default for
  # new clusters created through the GCP UIs.
  provider = google-beta

  name = "${var.resource_prefix}-cluster"
  # Specifying a region and not a zone here gives us a regional cluster, meaning
  # we get cluster masters across multiple zones.
  location    = var.gcp_region
  description = "Prio data share processor ${var.environment}"
  # We manage our own node pool below, so the next two parameters are required:
  # https://www.terraform.io/docs/providers/google/r/container_cluster.html#remove_default_node_pool
  remove_default_node_pool = true
  initial_node_count       = 1

  release_channel {
    channel = "REGULAR"
  }

  # We opt into a VPC native cluster because they have several benefits (see
  # https://cloud.google.com/kubernetes-engine/docs/how-to/alias-ips).
  networking_mode = "VPC_NATIVE"
  network         = google_compute_network.network.self_link
  subnetwork      = google_compute_subnetwork.subnet.self_link
  ip_allocation_policy {
    cluster_ipv4_cidr_block  = module.subnets.network_cidr_blocks["kubernetes_cluster"]
    services_ipv4_cidr_block = module.subnets.network_cidr_blocks["kubernetes_services"]
  }
  private_cluster_config {
    # Cluster nodes will only have private IP addresses and don't have a direct
    # route to the Internet.
    enable_private_nodes = true
    # Still give the control plane a public endpoint, which we need for
    # deployment and troubleshooting.
    enable_private_endpoint = false
    # Block to use for the control plane master endpoints. The control plane
    # resides on its own VPC, and this range will be peered with ours to
    # communicate with it.
    master_ipv4_cidr_block = module.subnets.network_cidr_blocks["kubernetes_control_plane"]
  }

  # Enables workload identity, which enables containers to authenticate as GCP
  # service accounts which may then be used to authenticate to AWS S3.
  # https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity
  workload_identity_config {
    identity_namespace = "${var.gcp_project}.svc.id.goog"
  }
  # This enables KMS encryption of the contents of the Kubernetes cluster etcd
  # instance which among other things, stores Kubernetes secrets, like the keys
  # used by data share processors to sign batches or decrypt ingestion shares.
  # https://cloud.google.com/kubernetes-engine/docs/how-to/encrypting-secrets
  database_encryption {
    state    = "ENCRYPTED"
    key_name = google_kms_crypto_key.etcd_encryption_key.id
  }

  # Enables boot integrity checking and monitoring for nodes in the cluster.
  # More configuration values are defined in node pools below.
  enable_shielded_nodes = true

  depends_on = [google_project_service.container]
}

resource "google_container_node_pool" "worker_nodes" {
  name               = "${var.resource_prefix}-node-pool"
  location           = var.gcp_region
  cluster            = google_container_cluster.cluster.name
  initial_node_count = var.cluster_settings.initial_node_count
  autoscaling {
    min_node_count = var.cluster_settings.min_node_count
    max_node_count = var.cluster_settings.max_node_count
  }
  node_config {
    disk_size_gb = "25"
    image_type   = "COS_CONTAINERD"
    machine_type = var.cluster_settings.gcp_machine_type
    oauth_scopes = [
      "storage-ro",
      "logging-write",
      "monitoring"
    ]
    # Configures nodes to obtain workload identity from GKE metadata service
    # https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity
    workload_metadata_config {
      node_metadata = "GKE_METADATA_SERVER"
    }

    shielded_instance_config {
      enable_secure_boot          = true
      enable_integrity_monitoring = true
    }
  }

  depends_on = [google_project_service.compute]
}

# KMS keyring to store etcd encryption key
resource "google_kms_key_ring" "keyring" {
  name = "${var.resource_prefix}-kms-keyring"
  # Keyrings can also be zonal, but ours must be regional to match the GKE
  # cluster.
  location = var.gcp_region

  depends_on = [google_project_service.kms]
}

# KMS key used by GKE cluster to encrypt contents of cluster etcd, crucially to
# protect Kubernetes secrets.
resource "google_kms_crypto_key" "etcd_encryption_key" {
  name     = "${var.resource_prefix}-etcd-encryption-key"
  key_ring = google_kms_key_ring.keyring.id
  purpose  = "ENCRYPT_DECRYPT"
  # Rotate database encryption key every 90 days. This doesn't reencrypt
  # existing secrets unless we go touch them.
  # https://cloud.google.com/kubernetes-engine/docs/how-to/encrypting-secrets#key_rotation
  rotation_period = "7776000s"
}

# We need the project _number_ to construct the GKE service account, below.
data "google_project" "project" {}

# Permit the GKE service account to use the KMS key. We construct the service
# account name per the specification in:
# https://cloud.google.com/kubernetes-engine/docs/how-to/encrypting-secrets#grant_permission_to_use_the_key
resource "google_kms_crypto_key_iam_binding" "etcd-encryption-key-iam-binding" {
  crypto_key_id = google_kms_crypto_key.etcd_encryption_key.id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  members = [
    "serviceAccount:service-${data.google_project.project.number}@container-engine-robot.iam.gserviceaccount.com"
  ]
}

data "google_client_config" "current" {}

output "cluster_name" {
  value = google_container_cluster.cluster.name
}

output "cluster_endpoint" {
  value = "https://${google_container_cluster.cluster.endpoint}"
}

output "certificate_authority_data" {
  value = google_container_cluster.cluster.master_auth.0.cluster_ca_certificate
}

output "kms_keyring" {
  value = google_kms_key_ring.keyring.id
}

output "token" {
  value = data.google_client_config.current.access_token
}
