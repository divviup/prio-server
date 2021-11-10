package test

import (
	"context"
	"fmt"

	"github.com/abetterinternet/prio-server/key-rotator/key"
	"github.com/abetterinternet/prio-server/key-rotator/storage"
)

// NewKey returns a Key which stores keys in memory only, suitable for testing.
func NewKey() Key {
	return Key{
		batchSigningKeys:     map[LocalityIngestor]key.Key{},
		packetEncryptionKeys: map[string]key.Key{},
	}
}

type Key struct {
	batchSigningKeys     map[LocalityIngestor]key.Key
	packetEncryptionKeys map[string]key.Key // locality -> key
}

// LocalityIngestor represents a (locality, ingestor) tuple.
type LocalityIngestor struct{ locality, ingestor string }

var _ storage.Key = Key{} // verify key satisfies storage.Key

// Standard storage.Key functions.
func (k Key) PutBatchSigningKey(ctx context.Context, locality, ingestor string, key key.Key) error {
	k.batchSigningKeys[LocalityIngestor{locality, ingestor}] = key
	return nil
}

func (k Key) PutPacketEncryptionKey(ctx context.Context, locality string, key key.Key) error {
	k.packetEncryptionKeys[locality] = key
	return nil
}

func (k Key) GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error) {
	bsk, ok := k.batchSigningKeys[LocalityIngestor{locality, ingestor}]
	if !ok {
		return key.Key{}, fmt.Errorf("no batch signing key stored for (%q, %q)", locality, ingestor)
	}
	return bsk, nil
}

func (k Key) GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error) {
	pek, ok := k.packetEncryptionKeys[locality]
	if !ok {
		return key.Key{}, fmt.Errorf("no packet encryption key stored for %q", locality)
	}
	return pek, nil
}

// Test-only functions.
func (k Key) BatchSigningKeys() map[LocalityIngestor]key.Key { return k.batchSigningKeys }

func (k Key) PacketEncryptionKeys() map[string]key.Key { return k.packetEncryptionKeys }
