// Package test provides in-memory intended-for-testing implementations of
// storage package interfaces.
package test

import (
	"context"
	"sync"

	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	"github.com/abetterinternet/prio-server/key-rotator/storage"
)

// NewManifest returns a Manifest which stores manifests in memory only,
// suitable for testing.
func NewManifest() *Manifest {
	return &Manifest{
		dspManifests: map[string]manifest.DataShareProcessorSpecificManifest{},
		dspPutCount:  map[string]int{},
	}
}

type Manifest struct {
	mu sync.Mutex // protects all fields

	dspManifests map[string]manifest.DataShareProcessorSpecificManifest
	dspPutCount  map[string]int

	ingestorManifest *manifest.IngestorGlobalManifest
	ingestorPutCount int
}

var _ storage.Manifest = &Manifest{} // verify *Manifest satisfies storage.Manifest

// Standard storage.Manifest functions. Safe for concurrent access from multiple goroutines.
func (m *Manifest) PutDataShareProcessorSpecificManifest(_ context.Context, dspName string, manifest manifest.DataShareProcessorSpecificManifest) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.dspManifests[dspName] = manifest
	m.dspPutCount[dspName]++
	return nil
}

func (m *Manifest) PutIngestorGlobalManifest(_ context.Context, manifest manifest.IngestorGlobalManifest) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.ingestorManifest = &manifest
	m.ingestorPutCount++
	return nil
}

func (m *Manifest) GetDataShareProcessorSpecificManifest(_ context.Context, dspName string) (manifest.DataShareProcessorSpecificManifest, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if manifest, ok := m.dspManifests[dspName]; ok {
		return manifest, nil
	}
	return manifest.DataShareProcessorSpecificManifest{}, storage.ErrObjectNotExist
}

func (m *Manifest) GetIngestorGlobalManifest(ctx context.Context) (manifest.IngestorGlobalManifest, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.ingestorManifest != nil {
		return *m.ingestorManifest, nil
	}
	return manifest.IngestorGlobalManifest{}, storage.ErrObjectNotExist
}

// Test-only functions. NOT goroutine-safe.
func (m *Manifest) GetDataShareProcessorSpecificManifests() map[string]manifest.DataShareProcessorSpecificManifest {
	return m.dspManifests
}

func (m *Manifest) GetDataShareProcessorSpecificManifestPutCount(dspName string) int {
	return m.dspPutCount[dspName]
}

func (m *Manifest) GetIngestorGlobalManifestPutCount() int { return m.ingestorPutCount }
