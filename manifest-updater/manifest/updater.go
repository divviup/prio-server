package manifest

import (
	"cloud.google.com/go/storage"
	"context"
	"encoding/json"
	"fmt"
	log "github.com/sirupsen/logrus"
)

type Updater struct {
	log                    *log.Entry
	environmentName        string
	locality               string
	manifestBucketLocation string
	ingestors              []string
}

func NewUpdater(environmentName string, locality string, manifestBucketLocation string, ingestors []string) (*Updater, error) {
	return &Updater{
		log:                    log.WithField("source", "manifest updater"),
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
		manifest, err := u.readExistingManifest(store, ingestor)
		if err != nil {
			return fmt.Errorf("manifest file not read properly: %w", err)
		}
		if keys != nil && keys[ingestor] != nil {
			manifest.BatchSigningPublicKeys = keys[ingestor]
		}
		if certificate != nil {
			manifest.PacketEncryptionKeyCSRs = certificate
		}

		err = u.writeManifest(store, manifest, ingestor)
		if err != nil {
			return fmt.Errorf("manifest file not written properly: %w", err)
		}
	}

	return nil
}

func (u *Updater) readExistingManifest(store *storage.BucketHandle, ingestor string) (*DataShareSpecificManifest, error) {
	u.log.WithField("ingestor", ingestor).Infoln("reading the manifest file")
	manifestObj := store.Object(fmt.Sprintf("%s-%s-manifest.json", u.locality, ingestor))
	r, err := manifestObj.NewReader(context.Background())
	if err != nil {
		return nil, fmt.Errorf("reading from the manifest failed: %w", err)
	}

	defer func() {
		err := r.Close()
		if err != nil {
			u.log.Fatal(err)
		}
	}()

	manifest := &DataShareSpecificManifest{}
	err = json.NewDecoder(r).Decode(manifest)
	if err != nil {
		return nil, fmt.Errorf("decoding manifest json failed: %w", err)
	}

	return manifest, nil
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
