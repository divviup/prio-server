package key

import (
	"context"
	"fmt"

	secretmanager "cloud.google.com/go/secretmanager/apiv1"
	"google.golang.org/api/iterator"
	secretmanagerpb "google.golang.org/genproto/googleapis/cloud/secretmanager/v1"
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
		return nil, fmt.Errorf("failed to create GCP Secrets Manager client: %w", err)
	}
	return &GCPSecretManagerBackup{
		client:      client,
		projectName: projectName,
	}, nil
}

func (b *GCPSecretManagerBackup) secretName(name string) string {
	return fmt.Sprintf("projects/%s/secrets/%s", b.projectName, name)
}

// ContainsSecret returns true if a secret with the provided name is already
// backed up, false if not, and an error if the secret's existence could not be
// determined.
func (b *GCPSecretManagerBackup) ContainsSecret(name string) (bool, error) {
	// GCP Secrets Manager has a notion of a *secret* which may contain multiple
	// *secret versions*, which actually contain secret values. We create the
	// secret in Terraform and so assume it exists, and check whether it
	// contains at least one ENABLED version, which we assume to be a valid
	// backup if it exists.
	request := secretmanagerpb.ListSecretVersionsRequest{
		Parent: b.secretName(name),
	}

	versionsIterator := b.client.ListSecretVersions(context.Background(), &request)

	for {
		version, err := versionsIterator.Next()
		if err == iterator.Done {
			return false, nil
		} else if err != nil {
			return false, fmt.Errorf("failed to list versions of secret %s", b.secretName(name))
		}
		if version.GetState() == secretmanagerpb.SecretVersion_ENABLED {
			return true, nil
		}
	}
}

func (b *GCPSecretManagerBackup) BackupSecret(name string, value []byte) error {
	request := secretmanagerpb.AddSecretVersionRequest{
		Parent: b.secretName(name),
		Payload: &secretmanagerpb.SecretPayload{
			Data: value,
		},
	}

	if _, err := b.client.AddSecretVersion(context.Background(), &request); err != nil {
		return fmt.Errorf("failed to add secret version: %w", err)
	}

	return nil
}
