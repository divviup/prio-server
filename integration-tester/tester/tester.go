package tester

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"log"
	"net/http"
	"time"

	"github.com/abetterinternet/prio-server/manifest-updater/manifest"
	"github.com/abetterinternet/prio-server/workflow-manager/kubernetes"
	batchv1 "k8s.io/api/batch/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

const (
	LabelKey   = "type"
	LabelValue = "integration-tester"
)

type Tester struct {
	kubeClient         *kubernetes.Client
	namespace          string
	ingestorLabel      string
	ownManifestUrl     string
	phaManifestUrl     string
	facilManifestUrl   string
	serviceAccountName string

	facilitatorImage string
	pushGateway      string
	awsAccountId     string
}

func New(
	kubeConfigPath,
	namespace, ingestorLabel,
	ownManifestUrl, phaManifestUrl, facilManifestUrl,
	serviceAccountName, facilitatorImage,
	pushGateway, awsAccountId string,
	dryRun bool) (*Tester, error) {

	kubeClient, err := kubernetes.NewClient(namespace, kubeConfigPath, dryRun)

	if err != nil {
		return nil, fmt.Errorf("error creating a new kubernetes client: %v", err)
	}

	return &Tester{
		kubeClient,
		namespace, ingestorLabel,
		ownManifestUrl, phaManifestUrl, facilManifestUrl,
		serviceAccountName, facilitatorImage,
		pushGateway, awsAccountId,
	}, nil
}

func (t *Tester) Start() error {
	ownManifest, err := readIngestorGlobalManifests(t.ownManifestUrl)
	if err != nil {
		return err
	}
	phaManifest, err := readDataShareProcessorSpecificManifest(t.phaManifestUrl)
	if err != nil {
		return err
	}
	facilManifest, err := readDataShareProcessorSpecificManifest(t.facilManifestUrl)
	if err != nil {
		return err
	}

	batchSigningKey, err := t.getValidBatchSigningKey(ownManifest)
	if err != nil {
		return err
	}

	phaPacketEncryptionKey, err := getValidPacketEncryptionKey(phaManifest)
	if err != nil {
		return err
	}

	facilPacketEncryptionKey, err := getValidPacketEncryptionKey(facilManifest)
	if err != nil {
		return err
	}

	job := t.createJob(phaManifest, facilManifest, phaPacketEncryptionKey, facilPacketEncryptionKey, batchSigningKey)

	log.Println("Scheduling job...")
	scheduledJob, err := t.kubeClient.ScheduleJob(job)
	if err != nil {
		return fmt.Errorf("scheduling job failed: %v", err)
	}
	if scheduledJob != nil {
		log.Printf("\tscheduled job: %s\n", scheduledJob.Name)
	}

	err = t.purgeOldJobs()
	if err != nil {
		log.Printf("Error when purging old successful jobs: %v\n", err)
	} else {
		log.Println("Purging old jobs was successful")
	}

	return nil
}

// purgeOldJobs will remove all successful jobs from the kubernetes namespace
func (t *Tester) purgeOldJobs() error {
	labelSelector := fmt.Sprintf("%s=%s,ingestor=%s", LabelKey, LabelValue, t.ingestorLabel)
	fieldSelector := "status.successful=1"

	log.Printf("purging jobs with labelSelector: %s and fieldSelector: %s\n", labelSelector, fieldSelector)

	err := t.kubeClient.RemoveJobCollection(metav1.DeleteOptions{}, metav1.ListOptions{
		LabelSelector: labelSelector,
		FieldSelector: fieldSelector,
	})

	if err != nil {
		return fmt.Errorf("purging old successful jobs failed: %v", err)
	}

	return nil
}

func (t *Tester) createJob(
	phaManifest manifest.DataShareProcessorSpecificManifest,
	facilManifest manifest.DataShareProcessorSpecificManifest,
	phaPacketEncryptionPublicKey, facilPacketEncryptionPublicKey string,
	batchSigningKey *corev1.Secret) *batchv1.Job {

	trueP := true
	backOffLimit := int32(1)
	env := []corev1.EnvVar{

		{Name: "BATCH_SIGNING_PRIVATE_KEY",
			ValueFrom: &corev1.EnvVarSource{SecretKeyRef: &corev1.SecretKeySelector{
				LocalObjectReference: corev1.LocalObjectReference{Name: batchSigningKey.Name},
				Key:                  "secret_key",
			}}},
		{Name: "BATCH_SIGNING_PRIVATE_KEY_IDENTIFIER", Value: batchSigningKey.Name},

		{Name: "PHA_ECIES_PUBLIC_KEY", Value: phaPacketEncryptionPublicKey},
		{Name: "FACILITATOR_ECIES_PUBLIC_KEY", Value: facilPacketEncryptionPublicKey},

		{Name: "RUST_LOG", Value: "info"},
		{Name: "RUST_BACKTRACE", Value: "1"},
		{Name: "AWS_ACCOUNT_ID", Value: t.awsAccountId},
	}

	return &batchv1.Job{
		ObjectMeta: metav1.ObjectMeta{
			GenerateName: "integration-tester-facilitator-",
			Labels: map[string]string{
				LabelKey:   LabelValue,
				"ingestor": t.ingestorLabel,
			},
		},
		Spec: batchv1.JobSpec{
			BackoffLimit: &backOffLimit,
			Template: corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{
					RestartPolicy:                "Never",
					ServiceAccountName:           t.serviceAccountName,
					AutomountServiceAccountToken: &trueP,
					Containers: []corev1.Container{
						{
							Name:  "integration-tester",
							Image: t.facilitatorImage,
							Env:   env,
							Args: []string{
								"--pushgateway", t.pushGateway,
								"generate-ingestion-sample",
								"--own-identity", facilManifest.IngestionIdentity,
								"--own-output", facilManifest.IngestionBucket,
								"--peer-identity", phaManifest.IngestionIdentity,
								"--peer-output", phaManifest.IngestionBucket,
								"--aggregation-id", "kittens-seen",
								"--packet-count", "10",
								// These parameters get recorded in Avro messages but otherwise
								// do not affect any system behavior, so the values don't matter.
								"--batch-start-time", "1000000000",
								"--batch-end-time", "1000000100",
								"--dimension", "123",
								"--epsilon", "0.23",
							},
						},
					},
				},
			},
		},
	}
}

// readDataShareProcessorSpecificManifest retrieves a manifest.DataShareProcessorSpecificManifest from the given url
func readDataShareProcessorSpecificManifest(url string) (manifest.DataShareProcessorSpecificManifest, error) {
	specificManifest := manifest.DataShareProcessorSpecificManifest{}
	client := http.Client{Timeout: 10 * time.Second}

	r, err := client.Get(url)
	if err != nil {
		return specificManifest, fmt.Errorf("unable to get %s: %v", url, err)
	}
	defer r.Body.Close()

	err = json.NewDecoder(r.Body).Decode(&specificManifest)
	if err != nil {
		return specificManifest, fmt.Errorf("unable to decode body %s: %v", r.Body, err)
	}
	return specificManifest, err
}

// readDataShareProcessorSpecificManifest retrieves a manifest.IngestorGlobalManifest from the given url
func readIngestorGlobalManifests(url string) (manifest.IngestorGlobalManifest, error) {
	ingestorGlobalManifest := manifest.IngestorGlobalManifest{}
	client := http.Client{Timeout: 10 * time.Second}

	r, err := client.Get(url)
	if err != nil {
		return ingestorGlobalManifest, fmt.Errorf("unable to get %s: %v", url, err)
	}
	defer r.Body.Close()

	err = json.NewDecoder(r.Body).Decode(&ingestorGlobalManifest)
	if err != nil {
		return ingestorGlobalManifest, fmt.Errorf("unable to decode body %s: %v", r.Body, err)
	}
	return ingestorGlobalManifest, err
}

func getValidPacketEncryptionKey(manifest manifest.DataShareProcessorSpecificManifest) (string, error) {
	for _, value := range manifest.PacketEncryptionKeyCSRs {
		publicKey, err := getBase64PublicKeyFromCSR(value.CertificateSigningRequest)
		if err != nil {
			return "", fmt.Errorf("error when parsing the packet encryption csr: %v", err)
		}

		return publicKey, nil
	}
	return "", fmt.Errorf("no packet encryption certificate signing requests in manifest: %v", manifest)
}

func getBase64PublicKeyFromCSR(pemCsr string) (string, error) {
	pemBlock, rest := pem.Decode([]byte(pemCsr))
	if len(rest) != 0 {
		return "", fmt.Errorf("unable to pem.Decode the CSR")
	}

	if pemBlock.Type != "CERTIFICATE REQUEST" {
		return "", fmt.Errorf("csr not a certificate request")
	}

	csr, err := x509.ParseCertificateRequest(pemBlock.Bytes)
	if err != nil {
		return "", fmt.Errorf("error when parsing the certifcate request: %v", err)
	}
	ecdsaPublicKey, ok := csr.PublicKey.(*ecdsa.PublicKey)

	if !ok {
		return "", fmt.Errorf("certificate request public key was not an ecdsa.PublicKey")
	}
	publicKeyBytes := elliptic.Marshal(ecdsaPublicKey.Curve, ecdsaPublicKey.X, ecdsaPublicKey.Y)

	return base64.StdEncoding.EncodeToString(publicKeyBytes), nil
}

func (t *Tester) getValidBatchSigningKey(manifest manifest.IngestorGlobalManifest) (*corev1.Secret, error) {
	labelSelector := "type=batch-signing-key"
	secrets, err := t.kubeClient.GetSortedSecrets(labelSelector)
	if err != nil {
		return nil, err
	}
	secretMap := indexSecretsByName(secrets)
	for name := range manifest.BatchSigningPublicKeys {
		val, ok := secretMap[name]

		if ok {
			return &val, nil
		}
	}
	return nil, fmt.Errorf("unable to find a suitable batch signing key - manifest was: %v", manifest)
}

func indexSecretsByName(secrets []corev1.Secret) map[string]corev1.Secret {
	idx := map[string]corev1.Secret{}

	for _, secret := range secrets {
		idx[secret.Name] = secret
	}

	return idx
}
