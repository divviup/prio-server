package storage

import (
	"context"
	"fmt"

	"github.com/abetterinternet/prio-server/key-rotator/key"
)

// Key represents a store of Prio keys, with functionality to read & write keys
// from the store.
type Key interface {
	// PutBatchSigningKey writes the provided key as the batch signing key for
	// the given (locality, ingestor) tuple, or returns an error on failure.
	PutBatchSigningKey(ctx context.Context, locality, ingestor string, key key.Key) error

	// PutPacketEncryptionKey writes the provided key as the packet encryption
	// key for the given locality, or returns an error on failure.
	PutPacketEncryptionKey(ctx context.Context, locality string, key key.Key) error

	// GetBatchSigningKey gets the batch signing key for the given (locality,
	// ingestor) pair, or returns an error on failure.
	GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error)

	// GetPacketEncryptionKey gets the packet encryption key for the given
	// locality, or returns an error on failure.
	GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error)
}

// NewBackupKey returns a Key implementation that mirrors writes to a "backup"
// storage.Key. All reads are performed via the "main" storage.Key (the
// "backup" storage.Key will never be used to fulfill a read). To avoid the
// possiblity of writing a key to main storage without backing it up, writes
// are performed by writing to the "backup" storage first, followed by writing
// to the "main" storage.
func NewBackupKey(main, backup Key) Key { return backupKey{main, backup} }

type backupKey struct{ main, backup Key }

var _ Key = backupKey{} // verify backupKey satisfies Key

func (k backupKey) PutBatchSigningKey(ctx context.Context, locality, ingestor string, key key.Key) error {
	if err := k.backup.PutBatchSigningKey(ctx, locality, ingestor, key); err != nil {
		return fmt.Errorf("couldn't write to backup storage: %w", err)
	}
	if err := k.main.PutBatchSigningKey(ctx, locality, ingestor, key); err != nil {
		return fmt.Errorf("couldn't write to main storage: %w", err)
	}
	return nil
}

func (k backupKey) PutPacketEncryptionKey(ctx context.Context, locality string, key key.Key) error {
	if err := k.backup.PutPacketEncryptionKey(ctx, locality, key); err != nil {
		return fmt.Errorf("couldn't write to backup storage: %w", err)
	}
	if err := k.main.PutPacketEncryptionKey(ctx, locality, key); err != nil {
		return fmt.Errorf("couldn't write to main storage: %w", err)
	}
	return nil
}

func (k backupKey) GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error) {
	return k.main.GetBatchSigningKey(ctx, locality, ingestor)
}

func (k backupKey) GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error) {
	return k.main.GetPacketEncryptionKey(ctx, locality)
}

func batchSigningKeyName(env, locality, ingestor string) string {
	return fmt.Sprintf("%s-%s-%s-batch-signing-key", env, locality, ingestor)
}

func packetEncryptionKeyName(env, locality string) string {
	return fmt.Sprintf("%s-%s-ingestion-packet-decryption-key", env, locality)
}
