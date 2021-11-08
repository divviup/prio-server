// Package test provides in-memory intended-for-testing implementations of
// storage package interfaces.
package test

import (
	"context"

	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	"github.com/abetterinternet/prio-server/key-rotator/storage"
)

// NewManifest returns a Manifest which stores manifests in memory only,
// suitable for testing.
func NewManifest() *Manifest {
	return &Manifest{dspInfos: map[string]*dspInfo{}}
}

type Manifest struct {
	dspInfos map[string]*dspInfo

	ingestorManifest *manifest.IngestorGlobalManifest
	ingestorPutCount int
}

type dspInfo struct {
	manifest manifest.DataShareProcessorSpecificManifest
	putCount int
}

var _ storage.Manifest = &Manifest{} // verify *Manifest satisfies storage.Manifest

// PutDataShareProcessorSpecificManifest writes the provided manifest for
// the provided share processor name in the writer's backing storage, or
// returns an error on failure.
func (m *Manifest) PutDataShareProcessorSpecificManifest(_ context.Context, dspName string, manifest manifest.DataShareProcessorSpecificManifest) error {
	di := m.dspInfos[dspName]
	if di == nil {
		di = &dspInfo{}
		m.dspInfos[dspName] = di
	}
	di.manifest = manifest
	di.putCount++
	return nil
}

// PutIngestorGlobalManifest writes the provided manifest to the writer's
// backing storage, or returns an error on failure.
func (m *Manifest) PutIngestorGlobalManifest(_ context.Context, manifest manifest.IngestorGlobalManifest) error {
	m.ingestorManifest = &manifest
	m.ingestorPutCount++
	return nil
}

// GetDataShareProcessorSpecificManifest gets the specific manifest for the
// specified data share processor and returns it, if it exists and is
// well-formed. If the manifest does not exist, an error wrapping
// storage.ErrObjectNotExist will be returned.
func (m Manifest) GetDataShareProcessorSpecificManifest(_ context.Context, dspName string) (manifest.DataShareProcessorSpecificManifest, error) {
	if di := m.dspInfos[dspName]; di != nil {
		return di.manifest, nil
	}
	return manifest.DataShareProcessorSpecificManifest{}, storage.ErrObjectNotExist
}

// GetDataShareProcessorSpecificManifestPutCount returns how many times
// PutDataShareProcessorSpecificManifest has been called for the given data
// share processor name.
func (m Manifest) GetDataShareProcessorSpecificManifestPutCount(dspName string) int {
	if di := m.dspInfos[dspName]; di != nil {
		return di.putCount
	}
	return 0
}

// GetIngestorGlobalManifest gets the ingestor global manifest, if it
// exists and is well-formed. If the manifest does not exist, an error
// wrapping storage.ErrObjectNotExist will be returned.
func (m Manifest) GetIngestorGlobalManifest(ctx context.Context) (manifest.IngestorGlobalManifest, error) {
	if m.ingestorManifest != nil {
		return *m.ingestorManifest, nil
	}
	return manifest.IngestorGlobalManifest{}, storage.ErrObjectNotExist
}

// GetIngestorGlobalManifestPutCount returns how many times
// PutIngestorGlobalManifest has been called.
func (m Manifest) GetIngestorGlobalManifestPutCount() int { return m.ingestorPutCount }
