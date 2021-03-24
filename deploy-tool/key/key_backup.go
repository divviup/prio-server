package key

import (
	"context"
	"fmt"
	"log"

	secretmanager "cloud.google.com/go/secretmanager/apiv1"
	"google.golang.org/api/iterator"
	secretmanagerpb "google.golang.org/genproto/googleapis/cloud/secretmanager/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	k8sv1 "k8s.io/api/core/v1"
)

const (
	secretUIDLabel = "kubernetes_uid"
)

type containsSecretResult int

const (
	secretDoesNotExist containsSecretResult = iota
	secretExists
	secretAndVersionExist
)

// GCPSecretManagerBackup backs up secrets in GCP Secret Manager
type GCPSecretManagerBackup struct {
	client      *secretmanager.Client
	projectName string
}

// NewGCPSecretManagerBackup creates a new GCPSecretManagerBackup using the
// provided GCP project name.
func NewGCPSecretManagerBackup(projectName string) (*GCPSecretManagerBackup, error) {
	client, err := secretmanager.NewClient(context.Background())
	if err != nil {
		return nil, fmt.Errorf("failed to create GCP Secret Manager client: %w", err)
	}
	return &GCPSecretManagerBackup{
		client:      client,
		projectName: projectName,
	}, nil
}

func (b *GCPSecretManagerBackup) projectFragment() string {
	return fmt.Sprintf("projects/%s", b.projectName)
}

func (b *GCPSecretManagerBackup) secretName(name string) string {
	// GCP Secret Manager is not namespaced the same way Kubernetes is, so we
	// must make sure that the GCP secret names we use are unique across all
	// namespaces in all Prio environments, since multiple environments can back
	// up secrets to a single GCP project. Fortunately the k8s secret names
	// already include both the environment name and the namespace so no further
	// disambiguation is needed.
	return fmt.Sprintf("%s/secrets/%s", b.projectFragment(), name)
}

// containsSecret checks if a GCP secret corresponding to the provided
// Kuberentes secret exists and returns:
//   - secretDoesNotExist if the GCP secret does not exist at all
//   - secretExists if the GCP secret exists but no enabled secret version was
//	   found
//	 - secretAndVersionExist if the GCP secret exists and an enabled secret
//     version was found
//
// If something goes wrong or if a secret with an unexpected label is found,
// returns secretDoesNotExist and an error.
func (b *GCPSecretManagerBackup) containsSecret(ctxt context.Context, k8sSecret *k8sv1.Secret) (containsSecretResult, error) {
	// GCP Secret Manager has a *secret* resource to which labels may be
	// attached and which contains *secret versions* which contain the actual
	// secret values. First, we check if a secret with the expected name exists,
	// and then verify that it has the expected label.
	getSecretRequest := secretmanagerpb.GetSecretRequest{
		Name: b.secretName(k8sSecret.ObjectMeta.Name),
	}

	gcpSecret, err := b.client.GetSecret(ctxt, &getSecretRequest)
	if err != nil {
		if status.Code(err) == codes.NotFound {
			return secretDoesNotExist, nil
		}
		return secretDoesNotExist, fmt.Errorf("failed to check if GCP secret %s exists: %w", b.secretName(k8sSecret.ObjectMeta.Name), err)
	}

	label, ok := gcpSecret.Labels[secretUIDLabel]
	if !ok {
		return secretDoesNotExist, fmt.Errorf(
			"GCP secret %s does not have %s label - this should be resolved manually",
			gcpSecret.Name, secretUIDLabel,
		)
	}
	if label != string(k8sSecret.ObjectMeta.UID) {
		return secretDoesNotExist, fmt.Errorf(
			"label %s on GCP secret %s does not match Kubernetes secret UID %s - this should be resolved manually",
			secretUIDLabel, gcpSecret.Name, k8sSecret.ObjectMeta.UID,
		)
	}

	// Check if the secret contains at least one enabled version, which we
	// assume to be our backed up secret.
	listSecretVersionsRequest := secretmanagerpb.ListSecretVersionsRequest{
		Parent: gcpSecret.Name,
	}

	versionsIterator := b.client.ListSecretVersions(ctxt, &listSecretVersionsRequest)

	for {
		version, err := versionsIterator.Next()
		if err == iterator.Done {
			return secretExists, nil
		} else if err != nil {
			return secretDoesNotExist, fmt.Errorf("failed to list versions of secret %s", b.secretName(k8sSecret.ObjectMeta.Name))
		}
		if version.GetState() == secretmanagerpb.SecretVersion_ENABLED {
			return secretAndVersionExist, nil
		}
	}
}

// BackupSecret backs up the provided Kubernetes secret into GCP Secret Manager,
// if it is not already backed up.
func (b *GCPSecretManagerBackup) BackupSecret(ctxt context.Context, k8sSecret *k8sv1.Secret) error {
	exists, err := b.containsSecret(ctxt, k8sSecret)
	if err != nil {
		return err
	}
	if exists == secretAndVersionExist {
		log.Printf("secret %s already backed up", k8sSecret.ObjectMeta.Name)
		return nil
	}

	log.Printf("backing up secret %s", k8sSecret.ObjectMeta.Name)

	if exists == secretDoesNotExist {
		// We create the secret here and not in Terraform so that an accidental
		// `terraform destroy` will not delete key backups.
		createSecretRequest := secretmanagerpb.CreateSecretRequest{
			Parent:   b.projectFragment(),
			SecretId: k8sSecret.ObjectMeta.Name,
			Secret: &secretmanagerpb.Secret{
				// I can't believe it takes this many {} and & to set a constant
				// but check the reference:
				// https://cloud.google.com/secret-manager/docs/reference/libraries#client-libraries-usage-go
				Replication: &secretmanagerpb.Replication{
					Replication: &secretmanagerpb.Replication_Automatic_{
						Automatic: &secretmanagerpb.Replication_Automatic{},
					},
				},
				Labels: map[string]string{
					secretUIDLabel: string(k8sSecret.ObjectMeta.UID),
				},
			},
		}
		if _, err := b.client.CreateSecret(ctxt, &createSecretRequest); err != nil {
			return fmt.Errorf("failed to create secret %s: %w", b.secretName(k8sSecret.ObjectMeta.Name), err)
		}

		exists = secretExists
	}

	if exists != secretExists {
		return fmt.Errorf("unexpected containsSecretResult %v", exists)
	}

	request := secretmanagerpb.AddSecretVersionRequest{
		Parent: b.secretName(k8sSecret.ObjectMeta.Name),
		Payload: &secretmanagerpb.SecretPayload{
			Data: k8sSecret.Data["secret_key"],
		},
	}

	if _, err := b.client.AddSecretVersion(ctxt, &request); err != nil {
		return fmt.Errorf("failed to add secret version: %w", err)
	}

	return nil
}
