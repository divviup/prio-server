environment     = "gamlin-test"
gcp_region      = "us-west1"
gcp_project     = "gamlin-test"
machine_type    = "e2-standard-8"
localities      = ["ta-ta", "narnia", "gondor", "asgard"]
aws_region      = "us-east-1"
aws_profile     = "leuseast1"
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
peer_share_processor_manifest_base_url = "test-en-analytics.cancer.gov"
portal_server_manifest_base_url        = "manifest.dev.enpa-pha.io"
is_first                               = false
aggregation_period                     = "30m"
aggregation_grace_period               = "30m"
