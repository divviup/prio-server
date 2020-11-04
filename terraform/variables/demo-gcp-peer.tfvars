environment                = "demo-gcp-peer"
gcp_region                 = "us-west1"
gcp_project                = "prio-bringup-290620"
machine_type               = "e2-small"
localities                 = ["narnia", "gondor", "asgard"]
aws_region                 = "us-west-1"
manifest_domain            = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  ingestor-1 = "demo-gcp-peer.manifests.isrg-prio.org/ingestor-1"
  ingestor-2 = "demo-gcp-peer.manifests.isrg-prio.org/ingestor-2"
}
peer_share_processor_manifest_base_url = "demo-gcp.manifests.isrg-prio.org"
portal_server_manifest_base_url        = "demo-gcp-peer.manifests.isrg-prio.org/portal-server"
test_peer_environment = {
  env_with_ingestor    = "demo-gcp"
  env_without_ingestor = "demo-gcp-peer"
}
is_first = true
