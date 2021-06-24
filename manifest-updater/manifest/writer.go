package manifest

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"

	"cloud.google.com/go/storage"
	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/credentials"
	"github.com/aws/aws-sdk-go/aws/session"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/rs/zerolog/log"
)

// Writer writes manifests to some storage
type Writer interface {
	// WriteDataShareProcessorSpecificManifest writes the provided manifest to
	// the provided path in the writer's backing storage, or returns an error
	// on failure.
	WriteDataShareProcessorSpecificManifest(manifest DataShareProcessorSpecificManifest, path string) error
	// WriteIngestorGlobalManifest writes the provided manifest to the provided
	// path in the writer's backing storage, or returns an error on failure.
	WriteIngestorGlobalManifest(manifest IngestorGlobalManifest, path string) error
}

// Bucket specifies the cloud storage bucket where manifests are stored
type Bucket struct {
	// Bucket is the name of the bucket, without any URL scheme
	Bucket string `json:"bucket"`
	// AWSRegion is the region the bucket is in, if it is an S3 bucket
	AWSRegion string `json:"aws_region,omitempty"`
	// AWSProfile is the AWS CLI config profile that should be used to
	// authenticate to AWS, if the bucket is an S3 bucket
	AWSProfile string `json:"aws_profile,omitempty"`
}

// NewWriter creates an instance of the appropriate implementation of Writer for
// the provided
//
func NewWriter(bucket *Bucket) (Writer, error) {
	if bucket.AWSRegion != "" {
		return newS3Writer(bucket.Bucket, bucket.AWSRegion, bucket.AWSProfile)
	}
	return newGCSWriter(bucket.Bucket), nil
}

// GCSWriter is a Writer that writes manifests to a Google Cloud Storage bucket
type GCSWriter struct {
	manifestBucketLocation string
}

// newGCSWriter creates a GCSWriter that writes manifests to the specified GCS
// bucket
func newGCSWriter(manifestBucketLocation string) *GCSWriter {
	return &GCSWriter{
		manifestBucketLocation: manifestBucketLocation,
	}
}

func (w *GCSWriter) WriteIngestorGlobalManifest(manifest IngestorGlobalManifest, path string) error {
	return w.writeManifest(manifest, path)
}

func (w *GCSWriter) WriteDataShareProcessorSpecificManifest(manifest DataShareProcessorSpecificManifest, path string) error {
	return w.writeManifest(manifest, path)
}

func (w *GCSWriter) writeManifest(manifest interface{}, path string) error {
	log.Info().
		Str("path", path).
		Msg("writing a manifest file")

	ioWriter, err := w.getWriter(path)
	if err != nil {
		return fmt.Errorf("unable to get writer: %w", err)
	}

	err = json.NewEncoder(ioWriter).Encode(manifest)
	if err != nil {
		_ = ioWriter.Close()
		return fmt.Errorf("encoding manifest json failed: %w", err)
	}

	err = ioWriter.Close()
	if err != nil {
		return fmt.Errorf("writing manifest failed: %w", err)
	}

	return nil
}

func (w *GCSWriter) getWriter(path string) (*storage.Writer, error) {
	client, err := storage.NewClient(context.Background())
	if err != nil {
		return nil, fmt.Errorf("unable to create a new storage client from background credentials: %w", err)
	}

	bucket := client.Bucket(w.manifestBucketLocation)

	manifestObj := bucket.Object(path)

	ioWriter := manifestObj.NewWriter(context.Background())
	ioWriter.CacheControl = "no-cache"
	ioWriter.ContentType = "application/json; charset=UTF-8"

	return ioWriter, nil
}

// S3Writer is a Writer that writes manifests to an S3 bucket
type S3Writer struct {
	client         *s3.S3
	manifestBucket string
}

// NewS3Writer creates an S3Writer that writes manifests to the specified S3
// bucket
func newS3Writer(manifestBucket, region, profile string) (*S3Writer, error) {
	sess, err := session.NewSession()
	if err != nil {
		return nil, fmt.Errorf("making AWS session: %w", err)
	}

	config := aws.
		NewConfig().
		WithRegion(region).
		WithCredentials(credentials.NewSharedCredentials("", profile))
	s3Client := s3.New(sess, config)

	return &S3Writer{
		client:         s3Client,
		manifestBucket: manifestBucket,
	}, nil
}

func (w *S3Writer) WriteIngestorGlobalManifest(manifest IngestorGlobalManifest, path string) error {
	return w.writeManifest(manifest, path)
}

func (w *S3Writer) WriteDataShareProcessorSpecificManifest(manifest DataShareProcessorSpecificManifest, path string) error {
	return w.writeManifest(manifest, path)
}

func (w *S3Writer) writeManifest(manifest interface{}, path string) error {
	log.Info().
		Str("path", path).
		Str("bucket", w.manifestBucket).
		Msg("writing a manifest file")

	jsonManifest, err := json.Marshal(manifest)
	if err != nil {
		return fmt.Errorf("failed to marshal manifest to JSON: %w", err)
	}

	input := &s3.PutObjectInput{
		ACL:          aws.String(s3.BucketCannedACLPublicRead),
		Body:         aws.ReadSeekCloser(bytes.NewReader(jsonManifest)),
		Bucket:       aws.String(w.manifestBucket),
		Key:          aws.String(path),
		CacheControl: aws.String("no-cache"),
		ContentType:  aws.String("application/json; charset=UTF-8"),
	}

	if _, err := w.client.PutObject(input); err != nil {
		return fmt.Errorf("storage.PutObject: %w", err)
	}

	return nil
}
