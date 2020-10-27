package main

import (
	"encoding/json"
	"fmt"
	"github.com/abetterinternet/prio-server/terraform-parser/kubernetes"
	"github.com/abetterinternet/prio-server/terraform-parser/terraform"
	"log"
	"os"
)

func main() {
	var output terraform.Output

	if err := json.NewDecoder(os.Stdin).Decode(&output); err != nil {
		log.Fatalf("failed to parse the terraform output: %v", err)
	}

	configuration := make(map[string]kubernetes.TerraformData)

	for dataShareProcessorName, manifestWrapper := range output.SpecificManifests.Value {
		namespace := manifestWrapper.KubernetesNamespace

		_, exists := configuration[namespace]
		if exists {
			continue
		}

		metadata := kubernetes.Metadata{
			Name:      fmt.Sprintf("%s-data", namespace),
			Namespace: namespace,
		}

		spec := kubernetes.Spec{
			CertificateFQDN:     manifestWrapper.CertificateFQDN,
			HealthAuthorityName: dataShareProcessorName,
			ManifestBucket:      output.ManifestBucket.Value,
		}

		data := kubernetes.TerraformData{
			ApiVersion: "terraform.isrg-prio.com/v1",
			Kind:       "TerraformData",
			Metadata:   metadata,
			Spec:       spec,
		}

		configuration[namespace] = data
	}

	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "    ")

	for name, data := range configuration {
		if err := encoder.Encode(data); err != nil {
			log.Fatalf("Failed to encode the output for %s:%v", name, err)
		}
		// Print out the three newlines to make it easier to separate into multiple files if the admin needs that.
		if _, err := os.Stdout.WriteString("\n\n\n"); err != nil {
			log.Fatalf("Failed to write newlines to the output for %s:%v", name, err)
		}
	}
}
