locals {
  # Address block used by the GKE cluster and other regional resources.
  # Addresses in 10.0.0.0/8 outside of this block are reserved for future
  # expansion.
  cluster_subnet_block = "10.64.0.0/11"
}

# A VPC is a global resource in GCP, however, subnets inside it are regional
# and so are created for each region by the gke module. See gke/network.tf
resource "google_compute_network" "network" {
  # Add prefix to support existing multi-env GCP projects
  name = "${var.environment}-network"
  # We will always have to create a subnet for the cluster anyways, so the auto
  # networks would never be used. This frees up the upper half of the 10.0.0.0/8
  # block, where these auto subnets would otherwise be allocated.
  auto_create_subnetworks = false
}
