package manifest

import (
	"context"
	"encoding/json"
	"fmt"

	"cloud.google.com/go/storage"
)

type Updater struct {
	manifestBucket string // Name of the ManifestBucket
}

const GlobalManifestJson = "global-manifest.json"

func NewUpdater(manifestBucket string) *Updater {
	return &Updater{manifestBucket}
}

func (u *Updater) UpdateDataShareGlobalManifest(manifest DataShareGlobalManifest) error {
	data, err := json.Marshal(manifest)
	if err != nil {
		return err
	}
	return u.update(GlobalManifestJson, data)
}

func (u *Updater) UpdateIngestionServerGlobalManifest(manifest IngestionServerGlobalManifest, ingestor string) error {
	data, err := json.Marshal(manifest)
	if err != nil {
		return err
	}
	return u.update(fmt.Sprintf("%s/%s", ingestor, GlobalManifestJson), data)
}

func (u *Updater) UpdateDataShareSpecificManifest(manifest DataShareSpecificManifest, phaIngestorName string) error {
	data, err := json.Marshal(manifest)
	if err != nil {
		return err
	}
	return u.update(fmt.Sprintf("%s-manifest.json", phaIngestorName), data)
}

func (u *Updater) update(path string, data []byte) error {
	client, err := storage.NewClient(context.Background())
	if err != nil {
		return err
	}
	bucket := client.Bucket(u.manifestBucket)
	obj := bucket.Object(path)

	writer := obj.NewWriter(context.Background())

	_, err = writer.Write(data)
	if err != nil {
		return err
	}

	err = writer.Close()
	return err
}
