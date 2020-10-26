environment                = "jsha-fridemo"
gcp_region                 = "us-west1"
gcp_project                = "jsha-prio-bringup"
machine_type               = "e2-small"
peer_share_processor_names = ["jsha-peer-1", "jsha-pha-2"]
aws_region                 = "us-west-1"
manifest_domain            = "prio.crud.net"
managed_dns_zone = {
  name        = "prio-crud-net"
  gcp_project = "jsha-prio-bringup"
}
ingestors = {
  lotophagi = "storage.googleapis.com/jsha-demo-cadet-manifests/lotophagi"
  baku = "storage.googleapis.com/jsha-demo-cadet-manifests/baku"
}
peer_share_processor_manifest_domain = "storage.googleapis.com/jsha-demo-cadet-manifests/byron"
