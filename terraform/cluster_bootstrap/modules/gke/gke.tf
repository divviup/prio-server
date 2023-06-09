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
  service            = "compute.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "container" {
  service = "container.googleapis.com"
}

resource "google_project_service" "kms" {
  service = "cloudkms.googleapis.com"
}

# This service is needed by Cloud Operations for GKE.
resource "google_project_service" "logging" {
  service            = "logging.googleapis.com"
  disable_on_destroy = false
}

# This service will be required by key-rotator, after bootstrapping.
resource "google_project_service" "secretmanager" {
  service = "secretmanager.googleapis.com"
}

resource "google_container_cluster" "cluster" {
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
    workload_pool = "${var.gcp_project}.svc.id.goog"
  }
  # This enables KMS encryption of the contents of the Kubernetes cluster etcd
  # instance which among other things, stores Kubernetes secrets, like the keys
  # used by data share processors to sign batches or decrypt ingestion shares.
  # https://cloud.google.com/kubernetes-engine/docs/how-to/encrypting-secrets
  database_encryption {
    state    = "ENCRYPTED"
    key_name = "${google_kms_crypto_key.etcd_encryption_key.key_ring}/cryptoKeys/${google_kms_crypto_key.etcd_encryption_key.name}"
  }

  # Enables boot integrity checking and monitoring for nodes in the cluster.
  # More configuration values are defined in node pools below.
  enable_shielded_nodes = true

  depends_on = [
    google_project_service.container,
    google_project_service.kms
  ]
}

resource "google_container_node_pool" "worker_nodes" {
  name     = "${var.resource_prefix}-node-pool"
  location = var.gcp_region
  cluster  = google_container_cluster.cluster.name
  # Since this is a regional cluster, this gives us one non-spot node per zone.
  initial_node_count = 1
  autoscaling {
    min_node_count = 1
    max_node_count = 1
  }
  node_config {
    disk_size_gb = "25"
    disk_type    = "pd-standard"
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
      mode = "GKE_METADATA"
    }

    shielded_instance_config {
      enable_secure_boot          = true
      enable_integrity_monitoring = true
    }
  }

  depends_on = [google_project_service.compute]
  lifecycle {
    ignore_changes = [initial_node_count]
  }
}

resource "google_container_node_pool" "spot_worker_nodes" {
  name               = "${var.resource_prefix}-spot-node-pool"
  location           = var.gcp_region
  cluster            = google_container_cluster.cluster.name
  initial_node_count = var.cluster_settings.initial_node_count
  autoscaling {
    min_node_count = var.cluster_settings.min_node_count
    max_node_count = var.cluster_settings.max_node_count
  }
  node_config {
    spot = true
    taint {
      key = "divviup.org/spot-vm"
      # We don't really need a value for this taint, but GKE's API requires one.
      value  = true
      effect = "NO_SCHEDULE"
    }

    disk_size_gb = "25"
    disk_type    = "pd-standard"
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
      mode = "GKE_METADATA"
    }

    shielded_instance_config {
      enable_secure_boot          = true
      enable_integrity_monitoring = true
    }
  }

  depends_on = [google_project_service.compute]
  lifecycle {
    ignore_changes = [initial_node_count]
  }
}

resource "random_string" "kms_id" {
  length  = 8
  upper   = false
  numeric = false
  special = false
}

# KMS keyring to store etcd encryption key
resource "google_kms_key_ring" "keyring" {
  name = "${var.resource_prefix}-kms-keyring-${random_string.kms_id.result}"
  # Keyrings can also be zonal, but ours must be regional to match the GKE
  # cluster.
  location = var.gcp_region

  depends_on = [google_project_service.kms]

  lifecycle {
    ignore_changes = [name]
  }
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
  crypto_key_id = "${google_kms_crypto_key.etcd_encryption_key.key_ring}/cryptoKeys/${google_kms_crypto_key.etcd_encryption_key.name}"
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  members = [
    "serviceAccount:service-${data.google_project.project.number}@container-engine-robot.iam.gserviceaccount.com"
  ]
}

# Create artifact registry to store container images
resource "google_project_service" "artifact_registry" {
  service = "artifactregistry.googleapis.com"
}

resource "google_artifact_registry_repository" "artifact_registry" {
  provider = google-beta

  repository_id = "prio-server-docker"
  format        = "DOCKER"
  location      = var.gcp_region

  depends_on = [google_project_service.artifact_registry]
}

output "google_kms_key_ring_id" {
  value = google_kms_key_ring.keyring.id
}
