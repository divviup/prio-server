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

// NewStorage creates a new Storage based on the given bucket parameters.
func NewStorage(bucket *Bucket) (Storage, error) {
	var ds datastore
	switch {
	case strings.HasPrefix(bucket.URL, "gs://"):
		bkt := strings.TrimPrefix(bucket.URL, "gs://")
		gcs, err := storage.NewClient(context.TODO())
		if err != nil {
			return nil, fmt.Errorf("couldn't create GCS storage client: %w", err)
		}
		ds = gcsDatastore{gcs, bkt}

	case strings.HasPrefix(bucket.URL, "s3://"):
		bkt := strings.TrimPrefix(bucket.URL, "s3://")
		sess, err := session.NewSession()
		if err != nil {
			return nil, fmt.Errorf("couldn't create AWS session: %w", err)
		}
		config := aws.NewConfig().WithRegion(bucket.AWSRegion).WithCredentials(credentials.NewSharedCredentials("", bucket.AWSProfile))
		s3 := s3.New(sess, config)
		ds = s3Datastore{s3, bkt}

	default:
		return nil, fmt.Errorf("bad bucket URL %q", bucket.URL)
	}
	return Manifest{ds, bucket.KeyPrefix}, nil
}

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

// FetchDataShareProcessorSpecificManifest fetches the specific manifest for
// the specified data share processor and returns it, if it exists and is
// well-formed. Returns (nil, nil) if the manifest does not exist.
func (m Manifest) FetchDataShareProcessorSpecificManifest(dataShareProcessorName string) (*manifest.DataShareProcessorSpecificManifest, error) {
	key := m.keyFor(dataShareProcessorName)
	manifestBytes, err := m.ds.get(context.TODO(), key)
	if errors.Is(err, errObjectNotExist) {
		return nil, nil
	}
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

type gcsDatastore struct {
	gcs    *storage.Client
	bucket string
}

var _ datastore = gcsDatastore{} // verify gcsDatastore satsifies datastore.

func (ds gcsDatastore) get(ctx context.Context, key string) ([]byte, error) {
	r, err := ds.gcs.Bucket(ds.bucket).Object(key).NewReader(ctx)
	if err != nil {
		if err == storage.ErrObjectNotExist {
			err = errObjectNotExist
		}
		return nil, fmt.Errorf("couldn't retrieve gs://%s/%s: %w", ds.bucket, key, err)
	}
	defer r.Close() // errchecked Close is below
	objBytes, err := io.ReadAll(r)
	if err != nil {
		return nil, fmt.Errorf("couldn't read gs://%s/%s: %w", ds.bucket, key, err)
	}
	if err := r.Close(); err != nil {
		return nil, fmt.Errorf("couldn't close gs://%s/%s: %w", ds.bucket, key, err)
	}
	return objBytes, nil
}

func (ds gcsDatastore) put(ctx context.Context, key string, data []byte) error {
	// Canceling a write requires canceling the context, rather than calling
	// Close(). We therefore create a context we can cancel without affecting
	// anything else to ensure we don't leave a pending write around in case of
	// failure.
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	w := ds.gcs.Bucket(ds.bucket).Object(key).NewWriter(ctx)
	w.CacheControl = "no-cache"
	w.ContentType = "application/json; charset=UTF-8"

	if _, err := w.Write(data); err != nil {
		return fmt.Errorf("couldn't write gs://%s/%s: %w", ds.bucket, key, err)
	}
	if err := w.Close(); err != nil {
		return fmt.Errorf("couldn't close gs://%s/%s: %w", ds.bucket, key, err)
	}
	return nil
}

type s3Datastore struct {
	s3     *s3.S3
	bucket string
}

var _ datastore = s3Datastore{} // verify s3Datastore satisfies datastore.

func (ds s3Datastore) get(ctx context.Context, key string) ([]byte, error) {
	objOut, err := ds.s3.GetObjectWithContext(ctx, &s3.GetObjectInput{
		Bucket: aws.String(ds.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		if awsErr, ok := err.(awserr.Error); ok && awsErr.Code() == s3.ErrCodeNoSuchKey {
			err = errObjectNotExist
		}
		return nil, fmt.Errorf("couldn't retrieve s3://%s/%s: %w", ds.bucket, key, err)
	}
	r := objOut.Body
	defer r.Close() // errchecked Close is below
	objBytes, err := io.ReadAll(r)
	if err != nil {
		return nil, fmt.Errorf("couldn't read s3://%s/%s: %w", ds.bucket, key, err)
	}
	if err := r.Close(); err != nil {
		return nil, fmt.Errorf("couldn't close s3://%s/%s: %w", ds.bucket, key, err)
	}
	return objBytes, nil

}

func (ds s3Datastore) put(ctx context.Context, key string, data []byte) error {
	if _, err := ds.s3.PutObjectWithContext(ctx, &s3.PutObjectInput{
		ACL:          aws.String(s3.BucketCannedACLPublicRead),
		Body:         bytes.NewReader(data),
		Bucket:       aws.String(ds.bucket),
		Key:          aws.String(key),
		CacheControl: aws.String("no-cache"),
		ContentType:  aws.String("application/json; charset=UTF-8"),
	}); err != nil {
		return fmt.Errorf("couldn't write s3://%s/%s: %w", ds.bucket, key, err)
	}
	return nil
}

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
