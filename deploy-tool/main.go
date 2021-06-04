package main

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"
	"os/user"
	"path"
	"time"

	k8smetav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	k8scorev1 "k8s.io/client-go/kubernetes/typed/core/v1"
	_ "k8s.io/client-go/plugin/pkg/client/auth/gcp"
	"k8s.io/client-go/tools/clientcmd"

	"github.com/abetterinternet/prio-server/deploy-tool/key"
	"github.com/abetterinternet/prio-server/manifest-updater/manifest"
)

// This tool consumes the output of `terraform apply`, generating keys and then
// populating specific manifests and Kubernetes secrets with appropriate values.
// We do this in this tool because if we generated secrets via Terraform
// resources, the secret values would appear in the Terraform state file. The
// struct definitions here MUST be kept in sync with the output variable in
// terraform/modules/facilitator/facilitator.tf and the corresponding structs in
// facilitator/src/manifest.rs.

// SpecificManifestWrapper is a struct that wraps a specific manifest with some
// metadata inserted by Terraform
type SpecificManifestWrapper struct {
	IngestorName        string                                      `json:"ingestor-name"`
	KubernetesNamespace string                                      `json:"kubernetes-namespace"`
	CertificateFQDN     string                                      `json:"certificate-fqdn"`
	SpecificManifest    manifest.DataShareProcessorSpecificManifest `json:"specific-manifest"`
}

// TerraformOutput represents the JSON output from `terraform apply` or
// `terraform output --json`. This struct must match the output variables
// defined in terraform/main.tf, though it only need describe the output
// variables this program is interested in.
type TerraformOutput struct {
	ManifestBucket struct {
		Value manifest.Bucket
	} `json:"manifest_bucket"`
	// OwnManifestBaseURL is a URL without a scheme (https), that manifests can
	// be found in
	OwnManifestBaseURL struct {
		Value string
	} `json:"own_manifest_base_url"`
	SpecificManifests struct {
		Value map[string]SpecificManifestWrapper
	} `json:"specific_manifests"`
	HasTestEnvironment struct {
		Value bool
	} `json:"has_test_environment"`
	SingletonIngestor struct {
		Value *SingletonIngestor
	} `json:"singleton_ingestor"`
	AWSProfile struct {
		Value string
	} `json:"aws_profile"`
}

// GlobalIngestor defines the structure for the global fake ingestor (apple-like
// ingestor) to create a manifest for
type SingletonIngestor struct {
	AwsIamEntity              string `json:"aws_iam_entity"`
	GcpServiceAccountEmail    string `json:"gcp_service_account_email"`
	GcpServiceAccountID       string `json:"gcp_service_account_id"`
	TesterKubernetesNamespace string `json:"tester_kubernetes_namespace"`
	BatchSigningKeyName       string `json:"batch_signing_key_name"`
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
func generateAndDeployKeyPair(
	k8sSecretsClient k8scorev1.SecretInterface,
	namespace, keyName string,
	keyMarshaler privateKeyMarshaler,
) (*ecdsa.PrivateKey, error) {
	p256Key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("failed to generate P-256 key: %w", err)
	}

	marshaledPrivateKey, err := keyMarshaler(p256Key)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal key to PKCS#8 document: %w", err)
	}

	log.Printf("updating Kubernetes secret %s/%s", namespace, keyName)
	secret, err := k8sSecretsClient.Get(context.Background(), keyName, k8smetav1.GetOptions{})
	if err != nil {
		return nil, fmt.Errorf("could not locate kubernetes secret %s/%s: %w", namespace, keyName, err)
	}

	secret.StringData = map[string]string{
		"secret_key": base64.StdEncoding.EncodeToString(marshaledPrivateKey),
	}

	if _, err := k8sSecretsClient.Update(context.Background(), secret, k8smetav1.UpdateOptions{}); err != nil {
		return nil, fmt.Errorf("failed to update kubernetes secret %s/%s: %w", namespace, keyName, err)
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

func setupTestEnvironment(
	k8sClient *kubernetes.Clientset,
	ingestor *SingletonIngestor,
	bucket *manifest.Bucket,
) error {
	batchSigningPublicKey, err := createBatchSigningPublicKey(
		k8sClient.CoreV1().Secrets(ingestor.TesterKubernetesNamespace),
		ingestor.TesterKubernetesNamespace,
		ingestor.BatchSigningKeyName,
	)
	if err != nil {
		return fmt.Errorf("error when creating the batch signing public key for the test environment")
	}

	globalManifest := manifest.IngestorGlobalManifest{
		Format: 1,
		ServerIdentity: manifest.ServerIdentity{
			AWSIamEntity:           ingestor.AwsIamEntity,
			GCPServiceAccountID:    ingestor.GcpServiceAccountID,
			GCPServiceAccountEmail: ingestor.GcpServiceAccountEmail,
		},
		BatchSigningPublicKeys: manifest.BatchSigningPublicKeys{
			ingestor.BatchSigningKeyName: *batchSigningPublicKey,
		},
	}

	writer, err := manifest.NewWriter(bucket)
	if err != nil {
		return err
	}

	return writer.WriteIngestorGlobalManifest(globalManifest, "singleton-ingestor/global-manifest.json")
}

func createBatchSigningPublicKey(
	k8sSecretsClient k8scorev1.SecretInterface,
	kubernetesNamespace, name string,
) (*manifest.BatchSigningPublicKey, error) {
	privateKey, err := generateAndDeployKeyPair(k8sSecretsClient, kubernetesNamespace, name, marshalPKCS8PrivateKey)
	if err != nil {
		return nil, fmt.Errorf("error generating and deploying key pair: %v", err)
	}

	pkixPublic, err := x509.MarshalPKIXPublicKey(privateKey.Public())
	if err != nil {
		return nil, fmt.Errorf("failed to marshal ECDSA public key to PKIX: %v", err)
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

	return &manifest.BatchSigningPublicKey{
		PublicKey:  string(pem.EncodeToMemory(&block)),
		Expiration: expiration,
	}, nil
}

func createManifest(
	k8sClient *kubernetes.Clientset,
	terraformOutput *TerraformOutput,
	dataShareProcessorName string,
	manifestWrapper *SpecificManifestWrapper,
) error {
	if manifestExists(terraformOutput.OwnManifestBaseURL.Value, dataShareProcessorName) {
		log.Printf("manifest for %s exists - ignoring", dataShareProcessorName)
		return nil
	}

	k8sSecretsClient := k8sClient.CoreV1().Secrets(manifestWrapper.KubernetesNamespace)

	newBatchSigningPublicKeys := manifest.BatchSigningPublicKeys{}
	for name, batchSigningPublicKey := range manifestWrapper.SpecificManifest.BatchSigningPublicKeys {
		if batchSigningPublicKey.PublicKey != "" {
			newBatchSigningPublicKeys[name] = batchSigningPublicKey
			continue
		}
		log.Printf("generating ECDSA P256 key %s", name)

		batchSigningPublicKey, err := createBatchSigningPublicKey(
			k8sSecretsClient,
			manifestWrapper.KubernetesNamespace,
			name,
		)
		if err != nil {
			return fmt.Errorf("error when creating batch signing public key: %v", err)
		}
		newBatchSigningPublicKeys[name] = *batchSigningPublicKey
	}

	manifestWrapper.SpecificManifest.BatchSigningPublicKeys = newBatchSigningPublicKeys

	certificatesByNamespace := manifest.PacketEncryptionKeyCSRs{}
	newCertificates := manifest.PacketEncryptionKeyCSRs{}
	for name, packetEncryptionCertificate := range manifestWrapper.SpecificManifest.PacketEncryptionKeyCSRs {
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
		privKey, err := generateAndDeployKeyPair(
			k8sSecretsClient,
			manifestWrapper.KubernetesNamespace,
			name,
			keyMarshaler,
		)
		if err != nil {
			return err
		}

		prioKey := key.NewPrioKey(privKey)
		csrTemplate := key.GetPrioCSRTemplate(manifestWrapper.CertificateFQDN)

		pemCSR, err := prioKey.CreatePemEncodedCertificateRequest(rand.Reader, csrTemplate)
		if err != nil {
			return err
		}

		packetEncryptionCertificate := manifest.PacketEncryptionCertificate{CertificateSigningRequest: pemCSR}

		certificatesByNamespace[manifestWrapper.KubernetesNamespace] = packetEncryptionCertificate
		newCertificates[name] = packetEncryptionCertificate
	}

	manifestWrapper.SpecificManifest.PacketEncryptionKeyCSRs = newCertificates

	// Put the specific manifests into the manifest bucket.
	destination := fmt.Sprintf("%s-manifest.json", dataShareProcessorName)
	writer, err := manifest.NewWriter(&terraformOutput.ManifestBucket.Value)
	if err != nil {
		return err
	}

	if err := writer.WriteDataShareSpecificManifest(manifestWrapper.SpecificManifest, destination); err != nil {
		return fmt.Errorf("could not write data share specific manifest: %s", err)
	}

	return nil
}

func backupKeys(
	k8sClient *kubernetes.Clientset,
	gcpProjectName string,
	manifestWrapper *SpecificManifestWrapper,
) error {
	context := context.Background()

	if gcpProjectName == "" {
		log.Printf("no GCP project name -- skipping key backup")
		return nil
	}
	// Build the list of batch signing and packet decryption keys that may need
	// to be backed up
	keyNames := []string{}

	for keyName := range manifestWrapper.SpecificManifest.BatchSigningPublicKeys {
		keyNames = append(keyNames, keyName)
	}

	for keyName := range manifestWrapper.SpecificManifest.PacketEncryptionKeyCSRs {
		keyNames = append(keyNames, keyName)
	}

	k8sSecretsClient := k8sClient.CoreV1().Secrets(manifestWrapper.KubernetesNamespace)
	secretsBackup, err := key.NewGCPSecretManagerBackup(gcpProjectName)
	if err != nil {
		return fmt.Errorf("failed to create secret backup client: %w", err)
	}

	for _, keyName := range keyNames {
		secret, err := k8sSecretsClient.Get(context, keyName, k8smetav1.GetOptions{})
		if err != nil {
			return fmt.Errorf("could not locate kubernetes secret %s/%s: %w", manifestWrapper.KubernetesNamespace, keyName, err)
		}

		if err := secretsBackup.BackupSecret(context, secret); err != nil {
			return fmt.Errorf("failed to backup secret %s: %w", secret.ObjectMeta.Name, err)
		}
	}

	return nil
}

func kubernetesClient(kubeConfigPath string) (*kubernetes.Clientset, error) {
	config, err := clientcmd.BuildConfigFromFlags("", kubeConfigPath)
	if err != nil {
		return nil, fmt.Errorf("failed to create Kubernetes client config: %w", err)
	}

	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, fmt.Errorf("failed to create Kubernetes client: %w", err)
	}

	return client, nil
}

var kubeConfigPath = flag.String("kube-config-path", "", "Path to Kubernetes config file to use for Kubernetes API requests. Default: ~/.kube/config")
var keyBackupGCPProject = flag.String("key-backup-gcp-project", "", "GCP project in which to store key backups.")

func main() {
	flag.Parse()

	if *kubeConfigPath == "" {
		currentUser, err := user.Current()
		if err != nil {
			log.Fatalf("failed to lookup current user: %s", err)
		}

		*kubeConfigPath = path.Join(currentUser.HomeDir, ".kube", "config")
	}

	var terraformOutput TerraformOutput

	if err := json.NewDecoder(os.Stdin).Decode(&terraformOutput); err != nil {
		log.Fatalf("failed to parse specific manifests: %v", err)
	}

	k8sClient, err := kubernetesClient(*kubeConfigPath)
	if err != nil {
		log.Fatalf("%s", err)
	}

	if terraformOutput.HasTestEnvironment.Value && terraformOutput.SingletonIngestor.Value != nil {
		if manifestExists(terraformOutput.OwnManifestBaseURL.Value, "singleton-ingestor/global") {
			log.Println("global ingestor manifest exists - ignoring")
		} else if err := setupTestEnvironment(
			k8sClient,
			terraformOutput.SingletonIngestor.Value,
			&terraformOutput.ManifestBucket.Value,
		); err != nil {
			log.Fatalf("%s", err)
		}
	}

	for dataShareProcessorName, manifestWrapper := range terraformOutput.SpecificManifests.Value {
		if err := createManifest(k8sClient, &terraformOutput, dataShareProcessorName, &manifestWrapper); err != nil {
			log.Fatalf("%s", err)
		}

		if err := backupKeys(k8sClient, *keyBackupGCPProject, &manifestWrapper); err != nil {
			log.Fatalf("%s", err)
		}
	}
}
