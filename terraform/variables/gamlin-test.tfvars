environment     = "gamlin-test"
gcp_region      = "us-west1"
gcp_project     = "gamlin-test"
machine_type    = "e2-standard-8"
localities      = ["narnia", "gondor", "asgard"]
aws_region      = "us-west-1"
manifest_domain = "isrg-prio.org"
managed_dns_zone = {
  name        = "manifests"
  gcp_project = "prio-bringup-290620"
}
ingestors = {
  apple = "exposure-notification.apple.com/manifest"
  # This is Google, but we aren't allowed to create GCS buckets with "google" in
  # their name
  g-enpa = "www.gstatic.com/prio-manifests"
}
peer_share_processor_manifest_base_url = "gamlin-test.manifests.isrg-prio.org/pha"
portal_server_manifest_base_url        = "gamlin-test.manifests.isrg-prio.org/portal-server"
is_first                               = false
