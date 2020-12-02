
variable "network" {
  description = "self_link of VPC which nodes will be connected to"
}

variable "base_subnet" {
  type        = string
  description = "CIDR block this region will use for its network subnets (assumed to be a /11)"
}

locals {
  # Expected size of base_subnet, used to calculate the size of subnets below
  subnet_prefix = 11
}

module "subnets" {
  source  = "hashicorp/subnets/cidr"
  version = "1.0.0"

  base_cidr_block = var.base_subnet
  networks = [
    {
      # Used to assign individual pods unique addresses. One /24 from this range
      # is assigned to each node, from where addresses for each each pod in it
      # will be  allocated.
      name = "kubernetes_cluster"
      # /14 - up to 1024 nodes (one /24 used per node)
      new_bits = 14 - local.subnet_prefix
    },
    {
      # Used to assign Kubernetes services their own addresses independent of
      # the pods they run on.
      name = "kubernetes_services"
      # /20 - up to 4096 services
      new_bits = 20 - local.subnet_prefix
    },
    {
      # Primary IPs for VM instances running the Kubernetes nodes.
      name = "vm_instances"
      # /20 - up to 4096 VMs
      new_bits = 20 - local.subnet_prefix
    },
    {
      # Addresses of the GKE Kubernetes control plane endpoints
      name = "kubernetes_control_plane"
      # /28 - The control plane subnet must be exactly a /28, no other sizes are
      #       accepted by the API.
      new_bits = 28 - local.subnet_prefix
    },
  ]
}

resource "google_compute_subnetwork" "subnet" {
  name    = "${var.resource_prefix}-${var.gcp_region}-instances"
  region  = var.gcp_region
  network = var.network

  ip_cidr_range = module.subnets.network_cidr_blocks["vm_instances"]
  # We'll let other resources automatically add the secondary address ranges

  # Needed so VMs in this network can access GCP APIs internally. Causes the
  # Google API public IPs to be short-circuited internally so we can connect to
  # APIs without going over the external network.
  private_ip_google_access = true
}

# We don't actually use any routing/BGP features of the Router, but one is
# required in order to configure a NAT gateway below.
resource "google_compute_router" "router" {
  name    = "${var.resource_prefix}-${var.gcp_region}-router"
  network = var.network
  region  = var.gcp_region
}

# This NAT gateway provides internet access to the Kubernetes cluster
resource "google_compute_router_nat" "nat" {
  name   = "${var.resource_prefix}-${var.gcp_region}-nat"
  router = google_compute_router.router.name

  # External IPs will be allocated and released as needed by the NAT according
  # to demand for ports. This can be changed and a pool manually managed if we
  # need to do any allow-listing of connections from the cluster in the future.
  nat_ip_allocate_option = "AUTO_ONLY"
  # Even though we only really need this for the GKE cluster, we have no use
  # case for segregated NAT gateways for different uses in the project, so it's
  # fine to make this gateway cover all subnets.
  source_subnetwork_ip_ranges_to_nat = "ALL_SUBNETWORKS_ALL_IP_RANGES"
}
