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

func batchSigningKeyName(env, locality, ingestor string) string {
	return fmt.Sprintf("%s-%s-%s-batch-signing-key", env, locality, ingestor)
}

func packetEncryptionKeyName(env, locality string) string {
	return fmt.Sprintf("%s-%s-ingestion-packet-decryption-key", env, locality)
}
