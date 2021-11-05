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

// ErrObjectNotExist is an error representing that an object did not exist.
var ErrObjectNotExist = errors.New("object does not exist")

// Manifest represents a store of manifests, with functionality to read & write
// manifests from the store.
type Manifest interface {
	// PutDataShareProcessorSpecificManifest writes the provided manifest for
	// the provided share processor name in the writer's backing storage, or
	// returns an error on failure.
	PutDataShareProcessorSpecificManifest(ctx context.Context, dataShareProcessorName string, manifest manifest.DataShareProcessorSpecificManifest) error

	// PutIngestorGlobalManifest writes the provided manifest to the writer's
	// backing storage, or returns an error on failure.
	PutIngestorGlobalManifest(ctx context.Context, manifest manifest.IngestorGlobalManifest) error

	// GetDataShareProcessorSpecificManifest gets the specific manifest for the
	// specified data share processor and returns it, if it exists and is
	// well-formed. If the manifest does not exist, an error wrapping
	// ErrObjectNotExist will be returned.
	GetDataShareProcessorSpecificManifest(ctx context.Context, dataShareProcessorName string) (manifest.DataShareProcessorSpecificManifest, error)

	// GetIngestorGlobalManifest gets the ingestor global manifest, if it
	// exists and is well-formed. If the manifest does not exist, an error
	// wrapping ErrObjectNotExist will be returned.
	GetIngestorGlobalManifest(ctx context.Context) (manifest.IngestorGlobalManifest, error)
}

// NewManifest creates a new Manifest based on the given bucket parameters. It
// will use the given bucket for storage, which should be in the format
// "gs://bucket_name" (to use GCS) or "s3://bucket_name" (to use S3).
func NewManifest(ctx context.Context, bucket string, opts ...ManifestOption) (Manifest, error) {
	var os manifestOpts
	for _, o := range opts {
		o(&os)
	}

	var kv kvStore
	switch {
	case strings.HasPrefix(bucket, "gs://"):
		bucket = strings.TrimPrefix(bucket, "gs://")
		gcs, err := storage.NewClient(ctx)
		if err != nil {
			return nil, fmt.Errorf("couldn't create GCS storage client: %w", err)
		}
		kv = gcsKVStore{gcs, bucket}

	case strings.HasPrefix(bucket, "s3://"):
		bucket = strings.TrimPrefix(bucket, "s3://")
		sess, err := session.NewSession()
		if err != nil {
			return nil, fmt.Errorf("couldn't create AWS session: %w", err)
		}
		config := aws.NewConfig().WithRegion(os.awsRegion).WithCredentials(credentials.NewSharedCredentials("", os.awsProfile))
		s3 := s3.New(sess, config)
		kv = s3KVStore{s3, bucket}

	default:
		return nil, fmt.Errorf("bad bucket URL %q", bucket)
	}
	return kvStoreManifest{kv, os.keyPrefix}, nil
}

type manifestOpts struct{ keyPrefix, awsRegion, awsProfile string }

// ManifestOption represents an option that can be passed to NewManifest.
type ManifestOption func(*manifestOpts)

// WithKeyPrefix returns a manifest option that sets a key prefix, which will
// be applied to all keys read or written from the underlying data store.
func WithKeyPrefix(keyPrefix string) ManifestOption {
	return func(opts *manifestOpts) { opts.keyPrefix = keyPrefix }
}

// WithAWSProfile returns a manifest option that sets the AWS profile to use.
// Applies only to Manifests backed by S3.
func WithAWSProfile(awsProfile string) ManifestOption {
	return func(opts *manifestOpts) { opts.awsProfile = awsProfile }
}

// WithAWSRegion returns a manifest option that sets the AWS region to use.
// Applies only to Manifests backed by S3.
func WithAWSRegion(awsRegion string) ManifestOption {
	return func(opts *manifestOpts) { opts.awsRegion = awsRegion }
}

// kvStoreManifest implements Manifest, and translates requests to some
// underlying key-value system.
type kvStoreManifest struct {
	kv        kvStore
	keyPrefix string
}

// ingestorGlobalManifestDataShareProcessorName is the special data share
// processor name used to denote the ingestor global manifest.
const ingestorGlobalManifestDataShareProcessorName = "global"

var _ Manifest = kvStoreManifest{} // verify kvStoreManifest satisfies Manifest

func (m kvStoreManifest) PutDataShareProcessorSpecificManifest(ctx context.Context, dataShareProcessorName string, manifest manifest.DataShareProcessorSpecificManifest) error {
	manifestBytes, err := json.Marshal(manifest)
	if err != nil {
		return fmt.Errorf("couldn't marshal manifest as JSON: %w", err)
	}
	key := m.keyFor(dataShareProcessorName)
	if err := m.kv.put(ctx, key, manifestBytes); err != nil {
		return fmt.Errorf("couldn't put manifest to %q: %w", key, err)
	}
	return nil
}

func (m kvStoreManifest) PutIngestorGlobalManifest(ctx context.Context, manifest manifest.IngestorGlobalManifest) error {
	manifestBytes, err := json.Marshal(manifest)
	if err != nil {
		return fmt.Errorf("couldn't marshal manifest as JSON: %w", err)
	}
	key := m.keyFor(ingestorGlobalManifestDataShareProcessorName)
	if err := m.kv.put(ctx, key, manifestBytes); err != nil {
		return fmt.Errorf("couldn't put manifest to %q: %w", key, err)
	}
	return nil
}

func (m kvStoreManifest) GetDataShareProcessorSpecificManifest(ctx context.Context, dataShareProcessorName string) (manifest.DataShareProcessorSpecificManifest, error) {
	key := m.keyFor(dataShareProcessorName)
	manifestBytes, err := m.kv.get(ctx, key)
	if err != nil {
		return manifest.DataShareProcessorSpecificManifest{}, fmt.Errorf("couldn't get manifest from %q: %w", key, err)
	}
	var dspsm manifest.DataShareProcessorSpecificManifest
	if err := json.Unmarshal(manifestBytes, &dspsm); err != nil {
		return manifest.DataShareProcessorSpecificManifest{}, fmt.Errorf("couldn't unmarshal manifest from JSON: %w", err)
	}
	return dspsm, nil
}

func (m kvStoreManifest) GetIngestorGlobalManifest(ctx context.Context) (manifest.IngestorGlobalManifest, error) {
	key := m.keyFor(ingestorGlobalManifestDataShareProcessorName)
	manifestBytes, err := m.kv.get(ctx, key)
	if err != nil {
		return manifest.IngestorGlobalManifest{}, fmt.Errorf("couldn't get manifest from %q: %w", key, err)
	}
	var igm manifest.IngestorGlobalManifest
	if err := json.Unmarshal(manifestBytes, &igm); err != nil {
		return manifest.IngestorGlobalManifest{}, fmt.Errorf("couldn't unmarshal manifest from JSON: %w", err)
	}
	return igm, nil
}

func (m kvStoreManifest) keyFor(dataShareProcessorName string) string {
	return path.Join(m.keyPrefix, fmt.Sprintf("%s-manifest.json", dataShareProcessorName))
}

// kvStore represents a given key/value object store backing a kvStoreManifest.
// It includes functionality for getting & putting individual objects by key,
// specialized for small objects (i.e. no streaming support).
type kvStore interface {
	// get gets the content of a given key, or returns an error if it can't.
	// If the key does not exist, an error wrapping ErrObjectNotExist is
	// returned.
	get(ctx context.Context, key string) ([]byte, error)

	// put puts the given content to the given key, or returns an error if it
	// can't.
	put(ctx context.Context, key string, data []byte) error
}

type gcsKVStore struct {
	gcs    *storage.Client
	bucket string
}

var _ kvStore = gcsKVStore{} // verify gcsDatastore satisfies kvStore.

func (kv gcsKVStore) get(ctx context.Context, key string) (_ []byte, retErr error) {
	r, err := kv.gcs.Bucket(kv.bucket).Object(key).NewReader(ctx)
	if err != nil {
		if err == storage.ErrObjectNotExist {
			err = ErrObjectNotExist
		}
		return nil, fmt.Errorf("couldn't retrieve gs://%s/%s: %w", kv.bucket, key, err)
	}
	defer func() {
		if err := r.Close(); err != nil {
			if retErr == nil {
				retErr = fmt.Errorf("couldn't close gs://%s/%s: %w", kv.bucket, key, err)
			}
		}
	}()
	objBytes, err := io.ReadAll(r)
	if err != nil {
		return nil, fmt.Errorf("couldn't read gs://%s/%s: %w", kv.bucket, key, err)
	}
	return objBytes, nil
}

func (kv gcsKVStore) put(ctx context.Context, key string, data []byte) error {
	// Canceling a write requires canceling the context, rather than calling
	// Close(). We therefore create a context we can cancel without affecting
	// anything else to ensure we don't leave a pending write around in case of
	// failure.
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	w := kv.gcs.Bucket(kv.bucket).Object(key).NewWriter(ctx)
	w.CacheControl = "no-cache"
	w.ContentType = "application/json; charset=UTF-8"

	if _, err := w.Write(data); err != nil {
		return fmt.Errorf("couldn't write gs://%s/%s: %w", kv.bucket, key, err)
	}
	if err := w.Close(); err != nil {
		return fmt.Errorf("couldn't close gs://%s/%s: %w", kv.bucket, key, err)
	}
	return nil
}

type s3KVStore struct {
	s3     *s3.S3
	bucket string
}

var _ kvStore = s3KVStore{} // verify s3KVStore satisfies kvStore.

func (kv s3KVStore) get(ctx context.Context, key string) (_ []byte, retErr error) {
	objOut, err := kv.s3.GetObjectWithContext(ctx, &s3.GetObjectInput{
		Bucket: aws.String(kv.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		if awsErr, ok := err.(awserr.Error); ok && awsErr.Code() == s3.ErrCodeNoSuchKey {
			err = ErrObjectNotExist
		}
		return nil, fmt.Errorf("couldn't retrieve s3://%s/%s: %w", kv.bucket, key, err)
	}
	r := objOut.Body
	defer func() {
		if err := r.Close(); err != nil {
			if retErr == nil {
				retErr = fmt.Errorf("couldn't close s3://%s/%s: %w", kv.bucket, key, err)
			}
		}
	}()
	objBytes, err := io.ReadAll(r)
	if err != nil {
		return nil, fmt.Errorf("couldn't read s3://%s/%s: %w", kv.bucket, key, err)
	}
	return objBytes, nil

}

func (ds s3KVStore) put(ctx context.Context, key string, data []byte) error {
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
