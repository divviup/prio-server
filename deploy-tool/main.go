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
	"net/http"
	"os"
	"os/exec"
	"sync"
	"time"

	"github.com/abetterinternet/prio-server/deploy-tool/key"
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
	// PKIX SubjectPublicKeyInfo structure. It must be a P-256 key.
	PublicKey string `json:"public-key"`
	// Expiration is the ISO 8601 encoded UTC date at which this key expires.
	Expiration string `json:"expiration"`
}

// PacketEncryptionCertificate represents a certificate containing a public key
// used for packet encryption.
type PacketEncryptionKey struct {
	// Certificate is the PEM armored X.509 certificate.
	CertificateSigningRequest string `json:"certificate-signing-request"`
}

// SpecificManifest represents the manifest file advertised by a data share
// processor. See the design document for the full specification.
// https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#heading=h.3j8dgxqo5h68
type SpecificManifest struct {
	// Format is the version of the manifest.
	Format int64 `json:"format"`
	// IngestionBucket is the region+name of the bucket that the data share
	// processor which owns the manifest reads ingestion batches from.
	IngestionBucket string `json:"ingestion-bucket"`
	// IngestionIdentity is the ARN of the AWS IAM role that should be assumed
	// by an ingestion server to write to this data share processor's ingestion
	// bucket, if the ingestor does not have an AWS account of their own.
	IngestionIdentity string `json:"ingestion-identity,omitempty"`
	// PeerValidationBucket is the region+name of the bucket that the data share
	// processor which owns the manifest reads peer validation batches from.
	PeerValidationBucket string `json:"peer-validation-bucket"`
	// BatchSigningPublicKeys maps key identifiers to batch signing public keys.
	// These are the keys that peers reading batches emitted by this data share
	// processor use to verify signatures.
	BatchSigningPublicKeys BatchSigningPublicKeys `json:"batch-signing-public-keys"`
	// PacketEncryptionCertificates maps key identifiers to packet encryption
	// certificates. The values are PEM encoded X.509 certificates, which
	// contain the public key corresponding to the private key that the data
	// share processor which owns the manifest uses to decrypt ingestion share
	// packets.
	PacketEncryptionKeys PacketEncryptionKeyCSRs `json:"packet-encryption-keys"`
}

// IngestorGlobalManifest represents the global manifest file for an ingestor
type IngestorGlobalManifest struct {
	// Format is the version of the manifest.
	Format int64 `json:"format"`
	// ServerIdentity represents the server identity for the advertising party of the manifest
	ServerIdentity ServerIdentity `json:"server-identity"`
	// BatchSigningPublicKeys maps key identifiers to batch signing public keys.
	// These are the keys that will be used to sign batches coming from this service
	BatchSigningPublicKeys BatchSigningPublicKeys `json:"batch-signing-public-keys"`
}

// ServerIdentity represents the server identity for the advertising party of the manifest
type ServerIdentity struct {
	// AwsIamEntity is ARN of user or role - apple only
	AwsIamEntity string `json:"aws-iam-entity"`
	// GcpServiceAccountID is the numeric unique service account ID
	GcpServiceAccountID string `json:"gcp-service-account-id"`
	// GcpServiceAccountEmail is the email address of the gcp service account
	GcpServiceAccountEmail string `json:"gcp-service-account-email"`
}

type BatchSigningPublicKeys = map[string]BatchSigningPublicKey
type PacketEncryptionKeyCSRs = map[string]PacketEncryptionKey

// TerraformOutput represents the JSON output from `terraform apply` or
// `terraform output --json`. This struct must match the output variables
// defined in terraform/main.tf, though it only need describe the output
// variables this program is interested in.
type TerraformOutput struct {
	ManifestBucket struct {
		Value string
	} `json:"manifest_bucket"`
	// OwnManifestBaseURL is a URL without a scheme (https), that manifests can be found in
	OwnManifestBaseURL struct {
		Value string
	} `json:"own_manifest_base_url"`
	SpecificManifests struct {
		Value map[string]struct {
			IngestorName        string           `json:"ingestor-name"`
			KubernetesNamespace string           `json:"kubernetes-namespace"`
			CertificateFQDN     string           `json:"certificate-fqdn"`
			SpecificManifest    SpecificManifest `json:"specific-manifest"`
		}
	} `json:"specific_manifests"`
	HasTestEnvironment struct {
		Value bool
	} `json:"has_test_environment"`
	SingletonIngestor struct {
		Value *SingletonIngestor
	} `json:"singleton_ingestor"`
}

// GlobalIngestor defines the structure for the global fake ingestor (apple-like ingestor) to create a manifest for
type SingletonIngestor struct {
	AwsIamEntity              string `json:"aws_iam_entity"`
	GcpServiceAccountEmail    string `json:"gcp_service_account_email"`
	GcpServiceAccountID       string `json:"gcp_service_account_id"`
	TesterKubernetesNamespace string `json:"tester_kubernetes_namespace"`
}

type privateKeyMarshaler func(*ecdsa.PrivateKey) ([]byte, error)

// marshalX962UncompressedPrivateKey encodes a P-256 private key into the format
// expected by libprio-rs encrypt::PrivateKey, which is the X9.62 uncompressed
// public key concatenated with the secret scalar.
func marshalX962UncompressedPrivateKey(ecdsaKey *ecdsa.PrivateKey) ([]byte, error) {
	marshaledPublicKey := elliptic.Marshal(elliptic.P256(), ecdsaKey.PublicKey.X, ecdsaKey.PublicKey.Y)
	return append(marshaledPublicKey, ecdsaKey.D.Bytes()...), nil
}

// marshalPKCS8PrivateKey encodes a P-256 private key into a PKCS#8 document.
// This function adapts x509.MarshalPKCS8PrivateKey to the privateKeyMarshaler
// type.
func marshalPKCS8PrivateKey(ecdsaKey *ecdsa.PrivateKey) ([]byte, error) {
	return x509.MarshalPKCS8PrivateKey(ecdsaKey)
}

// generateAndDeployKeyPair generates a P-256 key pair and stores the base64
// encoded PKCS#8 encoding of that key in a Kubernetes secret with the provided
// keyName, in the provided namespace. Returns the private key so the caller may
// use it to populate specific manifests.
func generateAndDeployKeyPair(namespace, keyName, ingestorName, keyType string, keyMarshaler privateKeyMarshaler) (*ecdsa.PrivateKey, error) {
	p256Key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("failed to generate P-256 key: %w", err)
	}

	marshaledPrivateKey, err := keyMarshaler(p256Key)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal key to PKCS#8 document: %w", err)
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

	base64PrivateKey := base64.StdEncoding.EncodeToString(marshaledPrivateKey)
	if err := ioutil.WriteFile(tempFile.Name(), []byte(base64PrivateKey), 0600); err != nil {
		return nil, fmt.Errorf("failed to write out PKCS#8 private key: %v", err)
	}

	secretArgument := fmt.Sprintf("--from-file=secret_key=%s", tempFile.Name())
	kubectlCreate := exec.Command("kubectl", "-n", namespace, "create",
		"secret", "generic", keyName, secretArgument, "--dry-run=client", "-o=json")

	kubectlLabel := exec.Command("kubectl", "label", "-f-", "--dry-run",
		"-o=json", "--local",
		fmt.Sprintf("type=%s", keyType), fmt.Sprintf("ingestor=%s", ingestorName))

	kubectlApply := exec.Command("kubectl", "apply", "-f", "-")

	read, write := io.Pipe()
	kubectlLabel.Stdin = read
	kubectlCreate.Stdout = write

	// Do this async because if we don't close `kubectl create`'s stdout,
	// `kubectl apply` will never make progress.
	go func() {
		defer write.Close()
		if err := kubectlCreate.Run(); err != nil {
			log.Fatalf("failed to run kubectl create: %v", err)
		}
	}()

	read2, write2 := io.Pipe()
	kubectlLabel.Stdout = write2
	kubectlApply.Stdin = read2
	go func() {
		defer write2.Close()
		if err := kubectlLabel.Run(); err != nil {
			log.Fatalf("failed to run kubectl create: %v", err)
		}
	}()

	if output, err := kubectlApply.CombinedOutput(); err != nil {
		return nil, fmt.Errorf("failed to run kubectl apply: %v\nCombined output: %s", err, output)
	}

	return p256Key, nil
}

func manifestExists(fqdn, dsp string) bool {
	// Remove the locality name from the FQDN
	path := fmt.Sprintf("https://%s/%s-manifest.json", fqdn, dsp)

	resp, err := http.Get(path)
	if err != nil {
		log.Fatalf("error when getting manifest %s: %s", path, err)
	}
	defer resp.Body.Close()

	return resp.StatusCode == 200
}

func setupTestEnvironment(ingestor *SingletonIngestor, manifestBucket string) {
	name := "integration-tester-signing-key"
	batchSigningPublicKey := createBatchSigningPublicKey(ingestor.TesterKubernetesNamespace, name, "")

	manifest := IngestorGlobalManifest{
		Format: 1,
		ServerIdentity: ServerIdentity{
			AwsIamEntity:           ingestor.AwsIamEntity,
			GcpServiceAccountID:    ingestor.GcpServiceAccountID,
			GcpServiceAccountEmail: ingestor.GcpServiceAccountEmail,
		},
		BatchSigningPublicKeys: map[string]BatchSigningPublicKey{
			name: batchSigningPublicKey,
		},
	}

	destination := fmt.Sprintf("gs://%s/singleton-ingestor/global-manifest.json", manifestBucket)

	log.Printf("uploading specific manifest %s", destination)
	gsutil := exec.Command("gsutil",
		"-h", "Content-Type:application/json",
		"-h", "Cache-Control:no-cache",
		"cp", "-", destination)
	stdin, err := gsutil.StdinPipe()
	if err != nil {
		log.Fatalf("could not get pipe to gsutil stdin: %v", err)
	}
	wg := sync.WaitGroup{}
	// We're going to need to sync once the goroutine below is complete
	wg.Add(1)

	// Do this async because if we don't close gsutil's stdin, it will
	// never be able to get started.
	go func() {
		defer stdin.Close()
		defer wg.Done()
		log.Printf("uploading manifest %+v", manifest)
		manifestEncoder := json.NewEncoder(stdin)
		if err := manifestEncoder.Encode(manifest); err != nil {
			log.Fatalf("failed to encode manifest into gsutil stdin: %v", err)
		}
	}()

	if output, err := gsutil.CombinedOutput(); err != nil {
		log.Fatalf("gsutil failed: %v\noutput: %s", err, output)
	}
	wg.Wait()
}

func createBatchSigningPublicKey(kubernetesNamespace, name, ingestorName string) BatchSigningPublicKey {
	privateKey, err := generateAndDeployKeyPair(kubernetesNamespace, name, ingestorName, "batch-signing-key", marshalPKCS8PrivateKey)
	if err != nil {
		log.Fatalf("%s", err)
	}

	pkixPublic, err := x509.MarshalPKIXPublicKey(privateKey.Public())
	if err != nil {
		log.Fatalf("failed to marshal ECDSA public key to PKIX: %v", err)
	}

	block := pem.Block{
		Type:  "PUBLIC KEY",
		Bytes: pkixPublic,
	}

	expiration := time.
		Now().
		AddDate(0, 0, 90). // key expires in 90 days
		UTC().
		Format(time.RFC3339)

	return BatchSigningPublicKey{
		PublicKey:  string(pem.EncodeToMemory(&block)),
		Expiration: expiration,
	}
}

func main() {
	wg := sync.WaitGroup{}
	var terraformOutput TerraformOutput

	if err := json.NewDecoder(os.Stdin).Decode(&terraformOutput); err != nil {
		log.Fatalf("failed to parse specific manifests: %v", err)
	}

	certificatesByNamespace := map[string]PacketEncryptionKey{}

	if terraformOutput.HasTestEnvironment.Value && terraformOutput.SingletonIngestor.Value != nil {
		setupTestEnvironment(terraformOutput.SingletonIngestor.Value, terraformOutput.ManifestBucket.Value)
		// it's fine if this overrides the previous data
	}

	for dataShareProcessorName, manifestWrapper := range terraformOutput.SpecificManifests.Value {
		if manifestExists(terraformOutput.OwnManifestBaseURL.Value, dataShareProcessorName) {
			log.Printf("manifest for %s exists - ignoring", dataShareProcessorName)
			continue
		}
		newBatchSigningPublicKeys := map[string]BatchSigningPublicKey{}
		for name, batchSigningPublicKey := range manifestWrapper.SpecificManifest.BatchSigningPublicKeys {
			if batchSigningPublicKey.PublicKey != "" {
				newBatchSigningPublicKeys[name] = batchSigningPublicKey
				continue
			}
			log.Printf("generating ECDSA P256 key %s", name)

			newBatchSigningPublicKeys[name] = createBatchSigningPublicKey(manifestWrapper.KubernetesNamespace, name, manifestWrapper.IngestorName)
		}

		manifestWrapper.SpecificManifest.BatchSigningPublicKeys = newBatchSigningPublicKeys

		newCertificates := map[string]PacketEncryptionKey{}
		for name, packetEncryptionCertificate := range manifestWrapper.SpecificManifest.PacketEncryptionKeys {
			if packetEncryptionCertificate.CertificateSigningRequest != "" {
				newCertificates[name] = packetEncryptionCertificate
				continue
			}

			// Packet encryption keys are shared among the data share processors
			// in a namespace, so avoid creating and certifying the key twice
			if certificate, ok := certificatesByNamespace[manifestWrapper.KubernetesNamespace]; ok {
				newCertificates[name] = certificate
				continue
			}

			log.Printf("generating and certifying P256 key %s", name)
			keyMarshaler := marshalX962UncompressedPrivateKey
			privKey, err := generateAndDeployKeyPair(manifestWrapper.KubernetesNamespace, name, manifestWrapper.IngestorName, "packet-decryption-key", keyMarshaler)
			if err != nil {
				log.Fatalf("%s", err)
			}

			prioKey := key.NewPrioKey(privKey)
			csrTemplate := key.GetPrioCSRTemplate(manifestWrapper.CertificateFQDN)

			pemCSR, err := prioKey.CreatePemEncodedCertificateRequest(rand.Reader, csrTemplate)
			if err != nil {
				log.Fatalf("%s", err)
			}

			packetEncryptionCertificate := PacketEncryptionKey{CertificateSigningRequest: pemCSR}

			certificatesByNamespace[manifestWrapper.KubernetesNamespace] = packetEncryptionCertificate
			newCertificates[name] = packetEncryptionCertificate
		}

		manifestWrapper.SpecificManifest.PacketEncryptionKeys = newCertificates

		// Put the specific manifests into the manifest bucket. Users of this
		// tool already need to have gsutil and valid Google Cloud credentials
		// to be able to use the Makefile, so execing out to gsutil saves us the
		// trouble of pulling in the gcloud SDK.
		destination := fmt.Sprintf("gs://%s/%s-manifest.json",
			terraformOutput.ManifestBucket.Value, dataShareProcessorName)
		log.Printf("uploading specific manifest %s", destination)
		gsutil := exec.Command("gsutil",
			"-h", "Content-Type:application/json",
			"-h", "Cache-Control:no-cache",
			"cp", "-", destination)
		stdin, err := gsutil.StdinPipe()
		if err != nil {
			log.Fatalf("could not get pipe to gsutil stdin: %v", err)
		}
		// We're going to need to sync once the goroutine below is complete
		wg.Add(1)

		// Do this async because if we don't close gsutil's stdin, it will
		// never be able to get started.
		go func() {
			defer stdin.Close()
			defer wg.Done()
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

	// Make sure everything can cleanly exit
	wg.Wait()
}
