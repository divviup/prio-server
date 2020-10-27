package terraform

// Output represents the JSON output from `terraform apply` or
// `terraform output --json`. This struct must match the output variables
// defined in terraform/main.tf, though it only need describe the output
// variables this program is interested in.
type Output struct {
	ManifestBucket    ManifestBucket    `json:"manifest_bucket"`
	SpecificManifests SpecificManifests `json:"specific_manifests"`
}

// ManifestBucket represents the ManifestBucket section of the Output
type ManifestBucket struct {
	Value string
}

// SpecificManifests is a K-V representation between the name of the
// ingestor and the SpecificManifestContainer
type SpecificManifests struct {
	Value map[string]SpecificManifestContainer `json:"value"`
}

// SpecificManifestContainer is the representation of the container that contains
// the SpecificManifest structure.
// We don't need the SpecificManifest so it is not included in this structure
type SpecificManifestContainer struct {
	KubernetesNamespace string `json:"kubernetes-namespace"`
	CertificateFQDN     string `json:"certificate-fqdn"`
}
