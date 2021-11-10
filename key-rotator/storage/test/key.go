package test

import (
	"context"
	"fmt"

	"github.com/abetterinternet/prio-server/key-rotator/key"
	"github.com/abetterinternet/prio-server/key-rotator/storage"
)

// NewKey returns a Key which stores keys in memory only, suitable for testing.
func NewKey() storage.Key {
	return inMemoryKey{
		batchSigningKeys:     map[localityIngestor]key.Key{},
		packetEncryptionKeys: map[string]key.Key{},
	}
}

type inMemoryKey struct {
	batchSigningKeys     map[localityIngestor]key.Key
	packetEncryptionKeys map[string]key.Key // locality -> key
}

type localityIngestor struct{ locality, ingestor string }

var _ storage.Key = inMemoryKey{} // verify key satisfies storage.Key

func (k inMemoryKey) PutBatchSigningKey(ctx context.Context, locality, ingestor string, key key.Key) error {
	k.batchSigningKeys[localityIngestor{locality, ingestor}] = key
	return nil
}

func (k inMemoryKey) PutPacketEncryptionKey(ctx context.Context, locality string, key key.Key) error {
	k.packetEncryptionKeys[locality] = key
	return nil
}

func (k inMemoryKey) GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error) {
	bsk, ok := k.batchSigningKeys[localityIngestor{locality, ingestor}]
	if !ok {
		return key.Key{}, fmt.Errorf("no batch signing key stored for (%q, %q)", locality, ingestor)
	}
	return bsk, nil
}

func (k inMemoryKey) GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error) {
	pek, ok := k.packetEncryptionKeys[locality]
	if !ok {
		return key.Key{}, fmt.Errorf("no packet encryption key stored for %q", locality)
	}
	return pek, nil
}
