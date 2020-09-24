variable "infra_name" {
  type = string
}

variable "resource_prefix" {
  type = string
}

variable "gcp_region" {
  type = string
}

variable "machine_type" {
  type = string
}

data "google_compute_zones" "available" {}

resource "google_container_cluster" "cluster" {
  name = "${var.resource_prefix}-cluster"
  # Specifying a region and not a zone here gives us a regional cluster, meaning
  # we get cluster masters across multiple zones. This could be overkill for our
  # availability requirements.
  location = var.gcp_region
  node_locations = [
    data.google_compute_zones.available.names[0],
    data.google_compute_zones.available.names[1],
    data.google_compute_zones.available.names[2]
  ]
  description = "Prio data share processor ${var.infra_name}"
  # https://www.terraform.io/docs/providers/google/r/container_cluster.html#remove_default_node_pool
  remove_default_node_pool = true
  initial_node_count       = 1
}

resource "google_container_node_pool" "worker_nodes" {
  name       = "${var.resource_prefix}-node-pool"
  location   = var.gcp_region
  cluster    = google_container_cluster.cluster.name
  node_count = 1
  autoscaling {
    min_node_count = 1
    max_node_count = 3
  }
  management {
    auto_repair  = true
    auto_upgrade = true
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
