package kubernetes

type TerraformData struct {
	ApiVersion string `json:"apiVersion"`
	Kind       string `json:"kind"`
	Metadata   `json:"metadata"`
	Spec       `json:"spec"`
}

type Metadata struct {
	Name      string `json:"name"`
	Namespace string `json:"namespace"`
}

type Spec struct {
	CertificateFQDN     string `json:"certificateFQDN"`
	HealthAuthorityName string `json:"healthAuthorityName"`
	ManifestBucket      string `json:"manifestBucket"`
}
