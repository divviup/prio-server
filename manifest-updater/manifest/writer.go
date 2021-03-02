package manifest

import (
	"context"
	"encoding/json"
	"fmt"

	"cloud.google.com/go/storage"
	"github.com/rs/zerolog/log"
)

type Writer struct {
	manifestBucketLocation string
}

func NewWriter(manifestBucketLocation string) Writer {
	return Writer{
		manifestBucketLocation: manifestBucketLocation,
	}
}

func (w *Writer) WriteIngestorGlobalManifest(manifest IngestorGlobalManifest, path string) error {
	log.Info().
		Str("path", path).
		Msg("writing the manifest file")

	iow, err := w.getWriter(path)
	if err != nil {
		return fmt.Errorf("unable to get writer: %w", err)
	}

	defer func() {
		err := iow.Close()
		if err != nil {
			log.Fatal().Err(err).Msg("Unable to write manifest")
		}
	}()

	err = json.NewEncoder(iow).Encode(manifest)
	if err != nil {
		return fmt.Errorf("encoding manifest json failed: %w", err)
	}

	return nil
}

func (w *Writer) WriteDataShareSpecificManifest(manifest DataShareProcessorSpecificManifest, path string) error {
	log.Info().
		Str("path", path).
		Msg("writing the manifest file")

	iow, err := w.getWriter(path)
	if err != nil {
		return fmt.Errorf("unable to get writer: %w", err)
	}

	defer func() {
		err := iow.Close()
		if err != nil {
			log.Fatal().Err(err).Msg("Unable to write manifest")
		}
	}()

	err = json.NewEncoder(iow).Encode(manifest)
	if err != nil {
		return fmt.Errorf("encoding manifest json failed: %w", err)
	}

	return nil
}

func (w *Writer) getWriter(path string) (*storage.Writer, error) {
	client, err := storage.NewClient(context.Background())
	if err != nil {
		return nil, fmt.Errorf("unable to create a new storage client from background credentials: %w", err)
	}

	bucket := client.Bucket(w.manifestBucketLocation)

	manifestObj := bucket.Object(path)

	iow := manifestObj.NewWriter(context.Background())
	iow.CacheControl = "no-cache"
	iow.ContentType = "application/json; charset=UTF-8"

	return iow, nil
}
