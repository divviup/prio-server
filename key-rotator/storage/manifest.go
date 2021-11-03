package storage

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"path"
	"strings"

	"cloud.google.com/go/storage"
	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/awserr"
	"github.com/aws/aws-sdk-go/aws/credentials"
	"github.com/aws/aws-sdk-go/aws/session"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/rs/zerolog/log"
)

// ingestorGlobalManifestDataShareProcessorName is the special data share
// processor name used to denote the ingestor global manifest.
const ingestorGlobalManifestDataShareProcessorName = "global"

// Manifest represents a store of manifests, with functionality to read & write
// manifests from the store.
type Manifest struct {
	ds        datastore
	keyPrefix string
}

var _ Storage = Manifest{}

// WriteDataShareProcessorSpecificManifest writes the provided manifest for
// the provided share processor name in the writer's backing storage, or
// returns an error on failure.
func (m Manifest) WriteDataShareProcessorSpecificManifest(manifest manifest.DataShareProcessorSpecificManifest, dataShareProcessorName string) error {
	manifestBytes, err := json.Marshal(manifest)
	if err != nil {
		return fmt.Errorf("couldn't marshal manifest as JSON: %w", err)
	}
	key := m.keyFor(dataShareProcessorName)
	if err := m.ds.put(context.TODO(), key, manifestBytes); err != nil {
		return fmt.Errorf("couldn't put manifest to %q: %w", key, err)
	}
	return nil
}

// WriteIngestorGlobalManifest writes the provided manifest to the writer's
// backing storage, or returns an error on failure.
func (m Manifest) WriteIngestorGlobalManifest(manifest manifest.IngestorGlobalManifest) error {
	manifestBytes, err := json.Marshal(manifest)
	if err != nil {
		return fmt.Errorf("couldn't marshal manifest as JSON: %w", err)
	}
	key := m.keyFor(ingestorGlobalManifestDataShareProcessorName)
	if err := m.ds.put(context.TODO(), key, manifestBytes); err != nil {
		return fmt.Errorf("couldn't put manifest to %q: %w", key, err)
	}
	return nil
}

// WriteDataShareProcessorSpecificManifest writes the provided manifest for
// the provided share processor name in the writer's backing storage, or
// returns an error on failure.
func (m Manifest) FetchDataShareProcessorSpecificManifest(dataShareProcessorName string) (*manifest.DataShareProcessorSpecificManifest, error) {
	key := m.keyFor(dataShareProcessorName)
	manifestBytes, err := m.ds.get(context.TODO(), key)
	if err != nil {
		return nil, fmt.Errorf("couldn't get manifest from %q: %w", key, err)
	}
	var manifest manifest.DataShareProcessorSpecificManifest
	if err := json.Unmarshal(manifestBytes, &manifest); err != nil {
		return nil, fmt.Errorf("couldn't unmarshal manifest from JSON: %w", err)
	}
	return &manifest, nil
}

// IngestorGlobalManifestExists returns true if the global manifest exists
// and is well-formed. Returns (false, nil) if it does not exist. Returns
// (false, error) if something went wrong while trying to fetch or parse the
// manifest.
func (m Manifest) IngestorGlobalManifestExists() (bool, error) {
	key := m.keyFor(ingestorGlobalManifestDataShareProcessorName)
	manifestBytes, err := m.ds.get(context.TODO(), key)
	if errors.Is(err, errObjectNotExist) {
		return false, nil
	}
	if err != nil {
		return false, fmt.Errorf("couldn't get manifest from %q: %w", key, err)
	}
	var manifest manifest.IngestorGlobalManifest
	if err := json.Unmarshal(manifestBytes, &manifest); err != nil {
		return false, fmt.Errorf("couldn't unmarshal manifest from JSON: %w", err)
	}
	return true, nil
}

func (m Manifest) keyFor(dataShareProcessorName string) string {
	return path.Join(m.keyPrefix, fmt.Sprintf("%s-manifest.json", dataShareProcessorName))
}

// datastore represents a given key/value object store backing a Manifest. It
// includes functionality for getting & putting individual objects by key,
// specialized for small objects (i.e. no streaming support).
type datastore interface {
	// get gets the content of a given key, or returns an error if it can't.
	// If the key does not exist, an error wrapping errObjectNotExist is
	// returned.
	get(ctx context.Context, key string) ([]byte, error)

	// put puts the given content to the given key, or returns an error if it
	// can't.
	put(ctx context.Context, key string, data []byte) error
}

// errObjectNotExist is an error representing that an object could not be retrieved from the datastore.
var errObjectNotExist = errors.New("object does not exist")

type Storage interface {
	Writer
	Fetcher
}

// Writer writes manifests to some storage
type Writer interface {
	// WriteDataShareProcessorSpecificManifest writes the provided manifest for
	// the provided share processor name in the writer's backing storage, or
	// returns an error on failure.
	WriteDataShareProcessorSpecificManifest(manifest manifest.DataShareProcessorSpecificManifest, dataShareProcessorName string) error

	// WriteIngestorGlobalManifest writes the provided manifest to the writer's
	// backing storage, or returns an error on failure.
	WriteIngestorGlobalManifest(manifest manifest.IngestorGlobalManifest) error
}

// Fetcher fetches manifests from some storage
type Fetcher interface {
	// FetchDataShareProcessorSpecificManifest fetches the specific manifest for
	// the specified data share processor and returns it, if it exists and is
	// well-formed. Returns (nil,  nil) if the  manifest does not exist.
	// Returns (nil, error) if something went wrong while trying to fetch or
	// parse the manifest.
	FetchDataShareProcessorSpecificManifest(dataShareProcessorName string) (*manifest.DataShareProcessorSpecificManifest, error)

	// IngestorGlobalManifestExists returns true if the global manifest exists
	// and is well-formed. Returns (false, nil) if it does not exist. Returns
	// (false, error) if something went wrong while trying to fetch or parse the
	// manifest.
	IngestorGlobalManifestExists() (bool, error)
}

// Bucket specifies the cloud storage bucket where manifests are stored
type Bucket struct {
	// URL is the URL of the bucket, with the scheme "gs" for GCS buckets or
	// "s3" for S3 buckets; e.g., "gs://bucket-name" or "s3://bucket-name"
	URL string `json:"bucket_url"`
	// KeyPrefix is a key prefix applied to every key read or written.
	KeyPrefix string
	// AWSRegion is the region the bucket is in, if it is an S3 bucket
	AWSRegion string `json:"aws_region,omitempty"`
	// AWSProfile is the AWS CLI config profile that should be used to
	// authenticate to AWS, if the bucket is an S3 bucket
	AWSProfile string `json:"aws_profile,omitempty"`
}

// NewStorage creates an instance of the appropriate implementation of Writer for
// the provided bucket
func NewStorage(bucket *Bucket) (Storage, error) {
	if strings.HasPrefix(bucket.URL, "gs://") {
		return newGCS(strings.TrimPrefix(bucket.URL, "gs://"), bucket.KeyPrefix)
	} else if strings.HasPrefix(bucket.URL, "s3://") {
		return newS3(strings.TrimPrefix(bucket.URL, "s3://"), bucket.KeyPrefix, bucket.AWSRegion, bucket.AWSProfile)
	} else {
		return nil, fmt.Errorf("bad bucket URL %s", bucket.URL)
	}
}

// GCSStorage is a Storage that stores manifests in a Google Cloud Storage bucket
type GCSStorage struct {
	client         *storage.Client
	manifestBucket string
	keyPrefix      string
}

// newGCS creates a GCSStorage
func newGCS(manifestBucketLocation, keyPrefix string) (*GCSStorage, error) {
	client, err := storage.NewClient(context.Background())
	if err != nil {
		return nil, fmt.Errorf("unable to create a new storage client from background credentials: %w", err)
	}
	return &GCSStorage{client, manifestBucketLocation, keyPrefix}, nil
}

func (s *GCSStorage) WriteIngestorGlobalManifest(manifest manifest.IngestorGlobalManifest) error {
	return s.writeManifest(manifest, "global-manifest.json")
}

func (s *GCSStorage) WriteDataShareProcessorSpecificManifest(manifest manifest.DataShareProcessorSpecificManifest, dataShareProcessorName string) error {
	return s.writeManifest(manifest, fmt.Sprintf("%s-manifest.json", dataShareProcessorName))
}

func (s *GCSStorage) writeManifest(manifest interface{}, path string) error {
	log.Info().
		Str("path", path).
		Msg("writing a manifest file")

	ioWriter := s.getWriter(path)

	if err := json.NewEncoder(ioWriter).Encode(manifest); err != nil {
		jsonErr := fmt.Errorf("encoding manifest to JSON failed: %w", err)

		if closeErr := ioWriter.Close(); closeErr != nil {
			return fmt.Errorf("failed to close manifest writer (with error: %s) while handling error: %w", closeErr, jsonErr)
		}

		return jsonErr
	}

	if err := ioWriter.Close(); err != nil {
		return fmt.Errorf("writing manifest failed: %w", err)
	}

	return nil
}

func (s *GCSStorage) getWriter(key string) *storage.Writer {
	ioWriter := s.client.Bucket(s.manifestBucket).
		Object(path.Join(s.keyPrefix, key)).
		NewWriter(context.Background())
	ioWriter.CacheControl = "no-cache"
	ioWriter.ContentType = "application/json; charset=UTF-8"

	return ioWriter
}

func (s *GCSStorage) FetchDataShareProcessorSpecificManifest(dataShareProcessorName string) (*manifest.DataShareProcessorSpecificManifest, error) {
	reader, err := s.getReader(fmt.Sprintf("%s-manifest.json", dataShareProcessorName))
	if err != nil {
		return nil, err
	}

	if reader == nil {
		return nil, nil
	}

	var manifest manifest.DataShareProcessorSpecificManifest
	if err := json.NewDecoder(reader).Decode(&manifest); err != nil {
		jsonErr := fmt.Errorf("error parsing manifest: %w", err)

		if closeErr := reader.Close(); err != nil {
			return nil, fmt.Errorf("failed to close manifest reader (with error: %s) while handling error: %w", closeErr, jsonErr)
		}

		return nil, jsonErr
	}

	if err := reader.Close(); err != nil {
		return nil, fmt.Errorf("closing manifest reader failed: %w", err)
	}

	return &manifest, nil
}

func (s *GCSStorage) IngestorGlobalManifestExists() (bool, error) {
	reader, err := s.getReader("global-manifest.json")
	if err != nil {
		return false, err
	}

	if reader == nil {
		return false, nil
	}

	var manifest manifest.IngestorGlobalManifest
	if err := json.NewDecoder(reader).Decode(&manifest); err != nil {
		jsonErr := fmt.Errorf("error parsing manifest: %w", err)

		if closeErr := reader.Close(); err != nil {
			return false, fmt.Errorf("failed to close manifest reader (with error: %s) while handling error: %w", closeErr, jsonErr)
		}

		return false, jsonErr
	}

	if err := reader.Close(); err != nil {
		return false, fmt.Errorf("failed to close manifest reader: %w", err)
	}

	return true, nil
}

// getReader returns a storage.Reader for the object at the provided path, if it
// exists. If it does not exist, returns (nil, nil). Returns an error if some
// error besides the object not existing occurs.
func (s *GCSStorage) getReader(key string) (*storage.Reader, error) {
	reader, err := s.client.Bucket(s.manifestBucket).
		Object(path.Join(s.keyPrefix, key)).
		NewReader(context.Background())
	if err != nil {
		if err == storage.ErrObjectNotExist {
			return nil, nil
		}

		return nil, fmt.Errorf("failed to get GCS object reader: %w", err)
	}

	return reader, nil
}

// S3Storage is a Storage that stores manifests in an S3 bucket
type S3Storage struct {
	client         *s3.S3
	manifestBucket string
	keyPrefix      string
}

// newS3 creates an S3Writer that stores manifests in the specified S3 bucket
func newS3(manifestBucket, keyPrefix, region, profile string) (*S3Storage, error) {
	sess, err := session.NewSession()
	if err != nil {
		return nil, fmt.Errorf("making AWS session: %w", err)
	}

	config := aws.
		NewConfig().
		WithRegion(region).
		WithCredentials(credentials.NewSharedCredentials("", profile))
	s3Client := s3.New(sess, config)

	return &S3Storage{
		client:         s3Client,
		manifestBucket: manifestBucket,
		keyPrefix:      keyPrefix,
	}, nil
}

func (s *S3Storage) WriteIngestorGlobalManifest(manifest manifest.IngestorGlobalManifest) error {
	return s.writeManifest(manifest, "global-manifest.json")
}

func (s *S3Storage) WriteDataShareProcessorSpecificManifest(manifest manifest.DataShareProcessorSpecificManifest, dataShareProcessorName string) error {
	return s.writeManifest(manifest, fmt.Sprintf("%s-manifest.json", dataShareProcessorName))
}

func (s *S3Storage) writeManifest(manifest interface{}, key string) error {
	log.Info().
		Str("path", key).
		Str("bucket", s.manifestBucket).
		Msg("writing a manifest file")

	jsonManifest, err := json.Marshal(manifest)
	if err != nil {
		return fmt.Errorf("failed to marshal manifest to JSON: %w", err)
	}

	if _, err := s.client.PutObject(&s3.PutObjectInput{
		ACL:          aws.String(s3.BucketCannedACLPublicRead),
		Body:         aws.ReadSeekCloser(bytes.NewReader(jsonManifest)),
		Bucket:       aws.String(s.manifestBucket),
		Key:          aws.String(path.Join(s.keyPrefix, key)),
		CacheControl: aws.String("no-cache"),
		ContentType:  aws.String("application/json; charset=UTF-8"),
	}); err != nil {
		return fmt.Errorf("storage.PutObject: %w", err)
	}
	return nil
}

func (s *S3Storage) FetchDataShareProcessorSpecificManifest(dataShareProcessorName string) (*manifest.DataShareProcessorSpecificManifest, error) {
	reader, err := s.getReader(fmt.Sprintf("%s-manifest.json", dataShareProcessorName))
	if err != nil {
		return nil, err
	}

	if reader == nil {
		return nil, nil
	}

	var manifest manifest.DataShareProcessorSpecificManifest
	if err := json.NewDecoder(reader).Decode(&manifest); err != nil {
		jsonErr := fmt.Errorf("error parsing manifest: %w", err)

		if closeErr := reader.Close(); err != nil {
			return nil, fmt.Errorf("failed to close manifest reader (with error: %s) while handling error: %w", closeErr, jsonErr)
		}

		return nil, jsonErr
	}

	return &manifest, nil
}

func (s *S3Storage) IngestorGlobalManifestExists() (bool, error) {
	reader, err := s.getReader("global-manifest.json")
	if err != nil {
		return false, err
	}

	if reader == nil {
		return false, nil
	}

	var manifest manifest.IngestorGlobalManifest
	if err := json.NewDecoder(reader).Decode(&manifest); err != nil {
		jsonErr := fmt.Errorf("error parsing manifest: %w", err)

		if closeErr := reader.Close(); err != nil {
			return false, fmt.Errorf("failed to close manifest reader (with error: %s) while handling error: %w", closeErr, jsonErr)
		}

		return false, jsonErr
	}

	if err := reader.Close(); err != nil {
		return false, fmt.Errorf("failed to close manifest reader: %w", err)
	}

	return true, nil
}

// getReader returns an object from which the object's contents may be read, if
// the object exists. The returned io.ReadCloser should be Close()d by the
// caller. Returns (nil, nil) if the object does not exist or an error if
// something else goes wrong.
func (s *S3Storage) getReader(key string) (io.ReadCloser, error) {
	input := &s3.GetObjectInput{
		Bucket: aws.String(s.manifestBucket),
		Key:    aws.String(path.Join(s.keyPrefix, key)),
	}

	output, err := s.client.GetObject(input)
	if err != nil {
		if awsErr, ok := err.(awserr.Error); ok && awsErr.Code() == s3.ErrCodeNoSuchKey {
			return nil, nil
		}
		return nil, fmt.Errorf("storage.GetObject: %w", err)
	}

	return output.Body, nil
}
