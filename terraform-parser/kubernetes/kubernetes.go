package kubernetes

// TerraformData is the representation of the CRD defined by the Operator
type TerraformData struct {
	ApiVersion string `json:"apiVersion"`
	Kind       string `json:"kind"`
	Metadata   `json:"metadata"`
	Spec       `json:"spec"`
}

// Metadata is the representation of the metadata of the CRD
type Metadata struct {
	Name      string `json:"name"`
	Namespace string `json:"namespace"`
}

// Spec is the representation of the spec of the CRD
type Spec struct {
	CertificateFQDN     string `json:"certificateFQDN"`
	HealthAuthorityName string `json:"healthAuthorityName"`
	ManifestBucket      string `json:"manifestBucket"`
}
