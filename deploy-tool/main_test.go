package main

import (
	"context"
	"reflect"
	"testing"

	k8scorev1 "k8s.io/api/core/v1"
	k8smetav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/apimachinery/pkg/watch"
	k8sapplyconfigurations "k8s.io/client-go/applyconfigurations/core/v1"
	k8stypedcorev1 "k8s.io/client-go/kubernetes/typed/core/v1"

	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	storagetest "github.com/abetterinternet/prio-server/key-rotator/storage/test"
)

var ctx = context.Background()

type FakeKubernetesSecretsClientGetter struct {
	secrets map[string]k8scorev1.Secret
}

func (g *FakeKubernetesSecretsClientGetter) Secrets(namespace string) k8stypedcorev1.SecretInterface {
	return &FakeKubernetesSecretsClient{g.secrets}
}

type FakeKubernetesSecretsClient struct {
	secrets map[string]k8scorev1.Secret
}

func (c *FakeKubernetesSecretsClient) Get(
	ctx context.Context,
	name string,
	opts k8smetav1.GetOptions,
) (*k8scorev1.Secret, error) {
	// We only ever get a k8s secret to update it with a real value for
	// secret_key, so don't bother filling any of its fields except for the
	// name, which we'll use to key into the map on Update
	return &k8scorev1.Secret{
		ObjectMeta: k8smetav1.ObjectMeta{
			Name: name,
		},
	}, nil
}

func (c *FakeKubernetesSecretsClient) Update(
	ctx context.Context,
	secret *k8scorev1.Secret,
	opts k8smetav1.UpdateOptions,
) (*k8scorev1.Secret, error) {
	c.secrets[secret.ObjectMeta.Name] = *secret
	return secret, nil
}

// Remaining methods are no-ops, needed only to satisfy the interface
func (c *FakeKubernetesSecretsClient) Create(
	ctx context.Context,
	secret *k8scorev1.Secret,
	opts k8smetav1.CreateOptions,
) (*k8scorev1.Secret, error) {
	return nil, nil
}

func (c *FakeKubernetesSecretsClient) Delete(
	ctx context.Context,
	name string,
	opts k8smetav1.DeleteOptions,
) error {
	return nil
}

func (c *FakeKubernetesSecretsClient) DeleteCollection(
	ctx context.Context,
	opts k8smetav1.DeleteOptions,
	listOpts k8smetav1.ListOptions,
) error {
	return nil
}

func (c *FakeKubernetesSecretsClient) List(
	ctx context.Context,
	opts k8smetav1.ListOptions,
) (*k8scorev1.SecretList, error) {
	return nil, nil
}

func (c *FakeKubernetesSecretsClient) Watch(
	ctx context.Context,
	opts k8smetav1.ListOptions,
) (watch.Interface, error) {
	return nil, nil
}

func (c *FakeKubernetesSecretsClient) Patch(
	ctx context.Context,
	name string,
	pt types.PatchType,
	data []byte,
	opts k8smetav1.PatchOptions,
	subresources ...string,
) (result *k8scorev1.Secret, err error) {
	return nil, nil
}

func (c *FakeKubernetesSecretsClient) Apply(
	ctx context.Context,
	secret *k8sapplyconfigurations.SecretApplyConfiguration,
	opts k8smetav1.ApplyOptions,
) (result *k8scorev1.Secret, err error) {
	return nil, nil
}
func TestCreateManifests(t *testing.T) {
	// Manifests described by Terraform output
	specificManifests := map[string]SpecificManifestWrapper{
		// This manifest represents an already deployed instance: there is a
		// corresponding manifest in the manifestFetcher with a packet
		// encryption key CSR and the packet encryption private key already
		// exists in Kubernetes secrets
		"manifest-already-posted": {
			IngestorName:        "ingestor-1",
			KubernetesNamespace: "packet-encryption-key-exists",
			CertificateFQDN:     "packet-encryption-key-exists.fake.tld",
			SpecificManifest: manifest.DataShareProcessorSpecificManifest{
				Format:               1,
				IngestionBucket:      "gs://irrelevant",
				PeerValidationBucket: "gs://irrelevant",
				BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
					"packet-encryption-key-exists-ingestor-1-batch-signing-key": {
						Expiration: "",
						PublicKey:  "",
					},
				},
				PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
					"packet-encryption-key-exists-packet-encryption-key": {
						CertificateSigningRequest: "",
					},
				},
			},
		},
		// This manifest represents an instance that is not yet deployed, but
		// shares a packet encryption key with an instance that is deployed
		"manifest-not-posted-encryption-key-exists": {
			IngestorName:        "ingestor-2",
			KubernetesNamespace: "packet-encryption-key-exists",
			CertificateFQDN:     "packet-encryption-key-exists.fake.tld",
			SpecificManifest: manifest.DataShareProcessorSpecificManifest{
				Format:               1,
				IngestionBucket:      "gs://irrelevant",
				PeerValidationBucket: "gs://irrelevant",
				BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
					"packet-encryption-key-exists-ingestor-2-batch-signing-key": {
						Expiration: "",
						PublicKey:  "",
					},
				},
				PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
					"packet-encryption-key-exists-packet-encryption-key": {
						CertificateSigningRequest: "",
					},
				},
			},
		},
		// This manifest is an instance for which neither the manifest nor the
		// packet encryption key is deployed.
		"manifest-not-posted-encryption-key-does-not-exist": {
			IngestorName:        "ingestor-1",
			KubernetesNamespace: "packet-encryption-key-does-not-exist",
			CertificateFQDN:     "packet-encryption-key-does-not-exist.fake.tld",
			SpecificManifest: manifest.DataShareProcessorSpecificManifest{
				Format:               1,
				IngestionBucket:      "gs://irrelevant",
				PeerValidationBucket: "gs://irrelevant",
				BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
					"packet-encryption-key-does-not-exist-ingestor-1-batch-signing-key": {
						Expiration: "",
						PublicKey:  "",
					},
				},
				PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
					"packet-encryption-key-does-not-exist-packet-encryption-key": {
						CertificateSigningRequest: "",
					},
				},
			},
		},
	}

	manifestAlreadyPosted := manifest.DataShareProcessorSpecificManifest{
		Format:               1,
		IngestionBucket:      "gs://irrelevant",
		PeerValidationBucket: "gs://irrelevant",
		BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
			"packet-encryption-key-exists-ingestor-1-batch-signing-key": {
				Expiration: "2021-09-21T22:49:45Z",
				PublicKey:  "fake-public-key",
			},
		},
		PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
			"packet-encryption-key-exists-packet-encryption-key": {
				CertificateSigningRequest: "fake-csr",
			},
		},
	}

	manifestStorage := storagetest.NewManifest()
	if err := manifestStorage.PutDataShareProcessorSpecificManifest(ctx, "manifest-already-posted", manifestAlreadyPosted); err != nil {
		t.Fatalf("Couldn't write preexisting manifest: %v", err)
	}

	secretsClientGetter := FakeKubernetesSecretsClientGetter{
		secrets: map[string]k8scorev1.Secret{},
	}

	if err := createManifests(ctx, &secretsClientGetter, specificManifests, manifestStorage); err != nil {
		t.Errorf("unexpected error %s", err)
	}

	if gotPutCount := manifestStorage.GetDataShareProcessorSpecificManifestPutCount("manifest-already-posted"); gotPutCount != 1 {
		t.Errorf("no manifest should be uploaded if manifest already existed (put count = %d, want %d)", gotPutCount, 1)
	}
	if _, ok := secretsClientGetter.secrets["packet-encryption-key-exists-ingestor-1-batch-signing-key"]; ok {
		t.Error("batch signing key should not be created if manifest already existed")
	}
	if _, ok := secretsClientGetter.secrets["packet-encryption-key-exists-packet-encryption-key"]; ok {
		t.Error("packet encryption key should not be created if it already existed")
	}

	manifest, err := manifestStorage.GetDataShareProcessorSpecificManifest(ctx, "manifest-not-posted-encryption-key-exists")
	if err != nil {
		t.Fatalf("Couldn't retrieve manifest for manifest-not-posted-encryption-key-exists: %v", err)
	}
	if !reflect.DeepEqual(manifest.PacketEncryptionKeyCSRs, manifestAlreadyPosted.PacketEncryptionKeyCSRs) {
		t.Error("CSRs in manifest should match existing packet encryption key")
	}
	if _, ok := secretsClientGetter.secrets["packet-encryption-key-exists-ingestor-2-batch-signing-key"]; !ok {
		t.Error("batch signing key should be created if manifest did not exist")
	}

	manifest, err = manifestStorage.GetDataShareProcessorSpecificManifest(ctx, "manifest-not-posted-encryption-key-does-not-exist")
	if err != nil {
		t.Fatalf("Couldn't retrieve manifest for manifest-not-posted-encryption-key-does-not-exist: %v", err)
	}
	if reflect.DeepEqual(manifest.PacketEncryptionKeyCSRs, manifestAlreadyPosted) {
		t.Error("new CSR should be generated for new packet encryption key")
	}
	if _, ok := secretsClientGetter.secrets["packet-encryption-key-does-not-exist-ingestor-1-batch-signing-key"]; !ok {
		t.Error("batch signing key should be created if manifest did not exist")
	}
	if _, ok := secretsClientGetter.secrets["packet-encryption-key-does-not-exist-packet-encryption-key"]; !ok {
		t.Error("packet encryption key should be created if manifest and packet encryption key did not exist")
	}
}

func TestCreateManifestsExistingDuplicatePacketEncryptionKeyCsrs(t *testing.T) {
	// Manifests described by Terraform output
	specificManifests := map[string]SpecificManifestWrapper{
		// This manifest represents an already deployed instance: there is a
		// corresponding manifest in the manifestFetcher with a packet
		// encryption key CSR and the packet encryption private key already
		// exists in Kubernetes secrets
		"manifest-already-posted-1": {
			IngestorName:        "ingestor-1",
			KubernetesNamespace: "packet-encryption-key-exists",
			CertificateFQDN:     "packet-encryption-key-exists.fake.tld",
			SpecificManifest: manifest.DataShareProcessorSpecificManifest{
				Format:               1,
				IngestionBucket:      "gs://irrelevant",
				PeerValidationBucket: "gs://irrelevant",
				BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
					"packet-encryption-key-exists-ingestor-1-batch-signing-key": {
						Expiration: "",
						PublicKey:  "",
					},
				},
				PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
					"packet-encryption-key-exists-packet-encryption-key": {
						CertificateSigningRequest: "",
					},
				},
			},
		},
		// This manifest represents an already deployed instance which shares a
		// packet encryption key with the previous manifest
		"manifest-already-posted-2": {
			IngestorName:        "ingestor-2",
			KubernetesNamespace: "packet-encryption-key-exists",
			CertificateFQDN:     "packet-encryption-key-exists.fake.tld",
			SpecificManifest: manifest.DataShareProcessorSpecificManifest{
				Format:               1,
				IngestionBucket:      "gs://irrelevant",
				PeerValidationBucket: "gs://irrelevant",
				BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
					"packet-encryption-key-exists-ingestor-2-batch-signing-key": {
						Expiration: "",
						PublicKey:  "",
					},
				},
				PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
					"packet-encryption-key-exists-packet-encryption-key": {
						CertificateSigningRequest: "",
					},
				},
			},
		},
	}

	manifestAlreadyPosted1 := manifest.DataShareProcessorSpecificManifest{
		Format:               1,
		IngestionBucket:      "gs://irrelevant",
		PeerValidationBucket: "gs://irrelevant",
		BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
			"packet-encryption-key-exists-ingestor-1-batch-signing-key": {
				Expiration: "2021-09-21T22:49:45Z",
				PublicKey:  "fake-public-key",
			},
		},
		PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
			"packet-encryption-key-exists-packet-encryption-key": {
				CertificateSigningRequest: "fake-csr-1",
			},
		},
	}
	manifestAlreadyPosted2 := manifest.DataShareProcessorSpecificManifest{
		Format:               1,
		IngestionBucket:      "gs://irrelevant",
		PeerValidationBucket: "gs://irrelevant",
		BatchSigningPublicKeys: map[string]manifest.BatchSigningPublicKey{
			"packet-encryption-key-exists-ingestor-1-batch-signing-key": {
				Expiration: "2021-09-21T22:49:45Z",
				PublicKey:  "fake-public-key",
			},
		},
		// Same key name as above but contents differ
		PacketEncryptionKeyCSRs: map[string]manifest.PacketEncryptionCertificate{
			"packet-encryption-key-exists-packet-encryption-key": {
				CertificateSigningRequest: "fake-csr-2",
			},
		},
	}

	manifestStorage := storagetest.NewManifest()
	if err := manifestStorage.PutDataShareProcessorSpecificManifest(ctx, "manifest-already-posted-1", manifestAlreadyPosted1); err != nil {
		t.Fatalf("Couldn't write first preexisting manifest: %v", err)
	}
	if err := manifestStorage.PutDataShareProcessorSpecificManifest(ctx, "manifest-already-posted-2", manifestAlreadyPosted2); err != nil {
		t.Fatalf("Couldn't write preexisting manifest: %v", err)
	}

	secretsClientGetter := FakeKubernetesSecretsClientGetter{
		secrets: map[string]k8scorev1.Secret{},
	}

	if err := createManifests(ctx, &secretsClientGetter, specificManifests, manifestStorage); err == nil {
		t.Error("manifest creation should fail when existing posted manifests contain two different CSRs for the same packet encryption key name")
	}
}
