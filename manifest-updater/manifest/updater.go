package manifest

import (
	"context"
	"encoding/json"
	"fmt"

	"cloud.google.com/go/storage"
	"github.com/abetterinternet/prio-server/manifest-updater/config"
	log "github.com/sirupsen/logrus"
)

type Updater struct {
	log                    *log.Entry
	config                 config.Config
	environmentName        string
	locality               string
	manifestBucketLocation string
	ingestors              []string
}

func NewUpdater(config config.Config, environmentName string, locality string, manifestBucketLocation string, ingestors []string) (*Updater, error) {
	return &Updater{
		log:                    log.WithField("source", "manifest updater"),
		config:                 config,
		environmentName:        environmentName,
		locality:               locality,
		manifestBucketLocation: manifestBucketLocation,
		ingestors:              ingestors,
	}, nil
}

func (u *Updater) UpdateDataShareSpecificManifest(keys map[string]BatchSigningPublicKeys, certificate PacketEncryptionKeyCSRs) error {
	if keys == nil && certificate == nil {
		return nil
	}

	store, err := u.getStorageClient()
	if err != nil {
		return fmt.Errorf("getting storage client failed: %w", err)
	}

	for _, ingestor := range u.ingestors {
		manifest := DataShareSpecificManifest{
			Format:                  1,
			IngestionBucket:         u.config.IngestionBuckets[ingestor],
			PeerValidationBucket:    u.config.PeerValidationBuckets[ingestor],
			BatchSigningPublicKeys:  keys[ingestor],
			PacketEncryptionKeyCSRs: certificate,
		}

		err = u.writeManifest(store, &manifest, ingestor)
		if err != nil {
			return fmt.Errorf("manifest file not written properly: %w", err)
		}
	}

	return nil
}

func (u *Updater) writeManifest(store *storage.BucketHandle, manifest *DataShareSpecificManifest, ingestor string) error {
	u.log.
		WithField("ingestor", ingestor).
		WithField("manifest", manifest).
		Infoln("writing the manifest file")

	manifestObj := store.Object(fmt.Sprintf("%s-%s-manifest.json", u.locality, ingestor))

	w := manifestObj.NewWriter(context.Background())
	w.CacheControl = "no-cache"
	w.ContentType = "application/json; charset=UTF-8"
	defer func() {
		err := w.Close()
		if err != nil {
			u.log.Fatal(err)
		}
	}()

	err := json.NewEncoder(w).Encode(manifest)
	if err != nil {
		return fmt.Errorf("encoding manifest json failed: %w", err)
	}
	return nil
}

func (u *Updater) getStorageClient() (*storage.BucketHandle, error) {
	client, err := storage.NewClient(context.Background())
	if err != nil {
		return nil, fmt.Errorf("unable to create a new storage client from background credentials: %w", err)
	}

	bucket := client.Bucket(u.manifestBucketLocation)
	return bucket, nil
}
