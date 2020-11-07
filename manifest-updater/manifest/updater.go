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
	dataShareProcessors    []string
}

func NewUpdater(environmentName string, locality string, manifestBucketLocation string, dataShareProcessors []string) (*Updater, error) {
	return &Updater{
		log:                    log.WithField("source", "manifest updater"),
		environmentName:        environmentName,
		locality:               locality,
		manifestBucketLocation: manifestBucketLocation,
		dataShareProcessors:    dataShareProcessors,
	}, nil
}

func (u *Updater) UpdateDataShareSpecificManifest(keys map[string]BatchSigningPublicKeys, certificate *PacketEncryptionCertificate) error {
	if keys == nil && certificate == nil {
		return nil
	}

	store, err := u.getStorageClient()
	if err != nil {
		return fmt.Errorf("getting storage client failed: %w", err)
	}

	for _, dsp := range u.dataShareProcessors {
		manifest, err := u.readExistingManifest(store, dsp)
		if err != nil {
			return fmt.Errorf("manifest file not read properly: %w", err)
		}
		if keys != nil {
			manifest.BatchSigningPublicKeys = keys[dsp]
		}
		if certificate != nil {
			manifest.PacketEncryptionCertificates = map[string]PacketEncryptionCertificate{u.constructPacketEncryptionLookupKey(dsp): *certificate}
		}

		err = u.writeManifest(store, manifest, dsp)
		if err != nil {
			return fmt.Errorf("manifest file not written properly: %w", err)
		}
	}

	return nil
}

func (u *Updater) readExistingManifest(store *storage.BucketHandle, dataShareProcessor string) (*DataShareSpecificManifest, error) {
	u.log.WithField("data share processor", dataShareProcessor).Infoln("reading the manifest file")
	manifestObj := store.Object(fmt.Sprintf("%s-%s-manifest.json", u.locality, dataShareProcessor))
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

func (u *Updater) writeManifest(store *storage.BucketHandle, manifest *DataShareSpecificManifest, dataShareProcessor string) error {
	u.log.
		WithField("data share processor", dataShareProcessor).
		WithField("manifest", manifest).
		Infoln("writing the manifest file")

	manifestObj := store.Object(fmt.Sprintf("%s-%s-manifest.json", u.locality, dataShareProcessor))

	w := manifestObj.NewWriter(context.Background())
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

func (u *Updater) constructBatchSigningLookupKey(dataShareProcessor string) string {
	return fmt.Sprintf("%s-%s-%s-batch-signing-key", u.environmentName, u.locality, dataShareProcessor)
}

func (u *Updater) constructPacketEncryptionLookupKey(dataShareProcessor string) string {
	return fmt.Sprintf("%s-%s-%s-ingestion-packet-decryption-key", u.environmentName, u.locality, dataShareProcessor)
}

func (u *Updater) getStorageClient() (*storage.BucketHandle, error) {
	client, err := storage.NewClient(context.Background())
	if err != nil {
		return nil, fmt.Errorf("unable to create a new storage client from background credentials: %w", err)
	}

	bucket := client.Bucket(u.manifestBucketLocation)
	return bucket, nil
}
