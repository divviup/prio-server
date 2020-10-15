package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"io/ioutil"
	"log"
	"os"
	"os/exec"
	"time"
)

// This tool consumes the output of `terraform apply`, generating keys and then
// populating specific manifests and Kubernetes secrets with appropriate values.
// We do this in this tool because if we generated secrets via Terraform
// resources, the secret values would appear in the Terraform state file. The
// struct definitions here MUST be kept in sync with the output variable in
// terraform/modules/facilitator/facilitator.tf and the corresponding structs in
// facilitator/src/manifest.rs.

// BatchSigningPublicKey represents a public key used for batch signing.
type BatchSigningPublicKey struct {
	// PublicKey is the PEM armored base64 encoding of the ASN.1 encoding of the
	// PKIX SubjectPublicKeyInfo structure. It must be an ECDSA P256 key.
	PublicKey string `json:"public-key"`
	// Expiration is the ISO 8601 encoded UTC date at which this key expires.
	Expiration string `json:"expiration"`
}

// PacketEncryptionCertificate represents a certificate containing a public key
// used for packet encryption.
type PacketEncryptionCertificate struct {
	// Certificate is the PEM armored X.509 certificate.
	Certificate string `json:"certificate"`
}

// SpecificManifest represents the manifest file advertised by a data share
// processor. See the design document for the full specification.
// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
type SpecificManifest struct {
	// Format is the version of the manifest.
	Format float64 `json:"format"`
	// IngestionBucket is the region+name of the bucket that the data share
	// processor which owns the manifest reads ingestion batches from.
	IngestionBucket string `json:"ingestion-bucket"`
	// PeerValidationBucket is the region+name of the bucket that the data share
	// processor which owns the manifest reads peer validation batches from.
	PeerValidationBucket string `json:"peer-validation-bucket"`
	// BatchSigningPublicKeys maps key identifiers to batch signing public keys.
	// These are the keys that peers reading batches emitted by this data share
	// processor use to verify signatures.
	BatchSigningPublicKeys map[string]BatchSigningPublicKey `json:"batch-signing-public-keys"`
	// PacketEncryptionCertificates maps key identifiers to packet encryption
	// certificates. The values are PEM encoded X.509 certificates, which
	// contain the public key corresponding to the private key that the data
	// share processor which owns the manifest uses to decrypt ingestion share
	// packets.
	PacketEncryptionCertificates map[string]PacketEncryptionCertificate `json:"packet-encryption-certificates"`
}

// TerraformOutput represents the JSON output from `terraform apply` or
// `terraform output --json`. This struct must match the output variables
// defined in terraform/main.tf, though it only need describe the output
// variables this program is interested in.
type TerraformOutput struct {
	ManifestBucket struct {
		Value string
	} `json:"manifest_bucket"`
	SpecificManifests struct {
		Value map[string]struct {
			KubernetesNamespace string           `json:"kubernetes-namespace"`
			SpecificManifest    SpecificManifest `json:"specific-manifest"`
		}
	} `json:"specific_manifests"`
}

func generateKeyPair(namespace, keyName string) (*ecdsa.PrivateKey, error) {
	ecdsaKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("failed to generate ECDSA P256 key: %w", err)
	}

	pkcs8PrivateKey, err := x509.MarshalPKCS8PrivateKey(ecdsaKey)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal ECDSA key to PKCS#8 document: %w", err)
	}

	// Put the private keys into Kubernetes secrets. There's no straightforward
	// way to update a secret value using kubectl, so we use the trick from[1].
	// There's no particular reason to use `-o=json` but we must set some output
	// format or nothing is written to stdout.
	// [1] https://blog.atomist.com/updating-a-kubernetes-secret-or-configmap/
	//
	// We can't provide the base64 encoding of the key to --from-literal because
	// then kubectl would base64 the base64, so we have to write the b64 to a
	// temp file and then provide that to --from-file.
	log.Printf("updating Kubernetes secret %s/%s", namespace, keyName)
	tempFile, err := ioutil.TempFile("", "pkcs8-private-key-")
	if err != nil {
		return nil, fmt.Errorf("failed to create temp file: %v", err)
	}
	defer tempFile.Close()
	defer os.Remove(tempFile.Name())

	base64PrivateKey := base64.StdEncoding.EncodeToString(pkcs8PrivateKey)
	if err := ioutil.WriteFile(tempFile.Name(), []byte(base64PrivateKey), 0644); err != nil {
		return nil, fmt.Errorf("failed to write out PKCS#8 private key: %v", err)
	}

	secretArgument := fmt.Sprintf("--from-file=signing_key=%s", tempFile.Name())
	kubectlCreate := exec.Command("kubectl", "-n", namespace, "create",
		"secret", "generic", keyName, secretArgument, "--dry-run=client", "-o=json")
	kubectlApply := exec.Command("kubectl", "apply", "-f", "-")
	read, write := io.Pipe()
	kubectlApply.Stdin = read
	kubectlCreate.Stdout = write

	// Do this async because if we don't close `kubectl create`'s stdout,
	// `kubectl apply` will never make progress.
	go func() {
		defer write.Close()
		if err := kubectlCreate.Run(); err != nil {
			log.Fatalf("failed to run kubectl create: %v", err)
		}
	}()

	if output, err := kubectlApply.CombinedOutput(); err != nil {
		return nil, fmt.Errorf("failed to run kubectl apply: %v\nCombined output: %s", err, output)
	}

	return ecdsaKey, nil
}

func main() {
	var terraformOutput TerraformOutput

	err := json.NewDecoder(os.Stdin).Decode(&terraformOutput)
	if err != nil {
		log.Fatalf("failed to parse specific manifests: %v", err)
	}

	for dataShareProcessorName, manifestWrapper := range terraformOutput.SpecificManifests.Value {
		newBatchSigningPublicKeys := map[string]BatchSigningPublicKey{}
		for name, batchSigningPublicKey := range manifestWrapper.SpecificManifest.BatchSigningPublicKeys {
			if batchSigningPublicKey.PublicKey != "" {
				newBatchSigningPublicKeys[name] = batchSigningPublicKey
				continue
			}
			log.Printf("generating ECDSA P256 key %s", name)
			privateKey, err := generateKeyPair(manifestWrapper.KubernetesNamespace, name)
			if err != nil {
				log.Fatalf("%s", err)
			}

			pkixPublic, err := x509.MarshalPKIXPublicKey(privateKey.Public())
			if err != nil {
				log.Fatalf("failed to marshal ECDSA public key to PKIX: %v", err)
			}

			block := pem.Block{
				Type:  "PUBLIC KEY",
				Bytes: []byte(pkixPublic),
			}

			newBatchSigningPublicKeys[name] = BatchSigningPublicKey{
				PublicKey:  string(pem.EncodeToMemory(&block)),
				Expiration: time.Now().UTC().Format(time.RFC3339),
			}
		}

		manifestWrapper.SpecificManifest.BatchSigningPublicKeys = newBatchSigningPublicKeys

		newCertificates := map[string]PacketEncryptionCertificate{}
		for name, packetEncryptionCertificate := range manifestWrapper.SpecificManifest.PacketEncryptionCertificates {
			if packetEncryptionCertificate.Certificate != "" {
				newCertificates[name] = packetEncryptionCertificate
				continue
			}
			log.Printf("generating and certifying P256 key %s", name)
			_, err := generateKeyPair(manifestWrapper.KubernetesNamespace, name)
			if err != nil {
				log.Fatalf("%s", err)
			}

			// TODO(timg) get certificate over the key, insert it here
			newCertificates[name] = PacketEncryptionCertificate{Certificate: "TODO get certificate"}
		}

		manifestWrapper.SpecificManifest.PacketEncryptionCertificates = newCertificates

		// Put the specific manifests into the manifest bucket. Users of this
		// tool already need to have gsutil and valid Google Cloud credentials
		// to be able to use the Makefile, so execing out to gsutil saves us the
		// trouble of pulling in the gcloud SDK.
		destination := fmt.Sprintf("gs://%s/%s.json",
			terraformOutput.ManifestBucket.Value, dataShareProcessorName)
		log.Printf("uploading specific manifest %s", destination)
		gsutil := exec.Command("gsutil", "-h",
			"Content-Type:application/json", "cp", "-", destination)
		stdin, err := gsutil.StdinPipe()
		if err != nil {
			log.Fatalf("could not get pipe to gsutil stdin: %v", err)
		}

		// Do this async becuase if we don't close gsutil's stdin, it will
		// never be able to get started.
		go func() {
			defer stdin.Close()
			log.Printf("uploading manifest %+v", manifestWrapper.SpecificManifest)
			manifestEncoder := json.NewEncoder(stdin)
			if err := manifestEncoder.Encode(manifestWrapper.SpecificManifest); err != nil {
				log.Fatalf("failed to encode manifest into gsutil stdin: %v", err)
			}
		}()

		if output, err := gsutil.CombinedOutput(); err != nil {
			log.Fatalf("gsutil failed: %v\noutput: %s", err, output)
		}
	}
}
