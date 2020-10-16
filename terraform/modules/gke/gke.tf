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

variable "machine_type" {
  type = string
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
  # We opt into a VPC native cluster because they have several benefits (see
  # https://cloud.google.com/kubernetes-engine/docs/how-to/alias-ips). Enabling
  # this networking_mode requires an ip_allocation_policy block and the
  # google-beta provider.
  provider        = google-beta
  networking_mode = "VPC_NATIVE"
  ip_allocation_policy {
    # We set these to blank values to let Terraform and Google choose
    # appropriate subnets for us. As we learn more and become more opinionated
    # about our network topology we can configure this explicitly.
    # https://www.terraform.io/docs/providers/google/r/container_cluster.html#ip_allocation_policy
    cluster_ipv4_cidr_block  = ""
    services_ipv4_cidr_block = ""
  }
  # Enables workload identity, which enables containers to authenticate as GCP
  # service accounts which may then be used to authenticate to AWS S3.
  # https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity
  workload_identity_config {
    identity_namespace = "${var.gcp_project}.svc.id.goog"
  }
}

resource "google_container_node_pool" "worker_nodes" {
  provider           = google-beta
  name               = "${var.resource_prefix}-node-pool"
  location           = var.gcp_region
  cluster            = google_container_cluster.cluster.name
  initial_node_count = 1
  autoscaling {
    min_node_count = 1
    max_node_count = 3
  }
  node_config {
    disk_size_gb = "25"
    image_type   = "COS"
    machine_type = var.machine_type
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
  }
}

output "cluster_name" {
  value = google_container_cluster.cluster.name
}

output "cluster_endpoint" {
  value = "https://${google_container_cluster.cluster.endpoint}"
}

output "certificate_authority_data" {
  value = google_container_cluster.cluster.master_auth.0.cluster_ca_certificate
}
