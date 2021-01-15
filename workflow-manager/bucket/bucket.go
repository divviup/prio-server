package bucket

import (
	"context"
	"fmt"
	"io"
	"log"
	"strings"
	"time"

	leaws "github.com/letsencrypt/prio-server/workflow-manager/aws"
	"github.com/letsencrypt/prio-server/workflow-manager/utils"

	"cloud.google.com/go/storage"
	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/service/s3"
	"google.golang.org/api/iterator"
)

// TaskMarkerWriter allows writing of a task marker to some storage
type TaskMarkerWriter interface {
	WriteTaskMarker(marker string) error
}

// Bucket represents a general bucket of data
type Bucket struct {
	// service is either "s3", or "gs"
	service string
	// bucketName includes the region for S3
	bucketName string
	identity   string
	dryRun     bool
}

// New creates a new Bucket from a URL and identity. If dryRun is true, then any
// operations with side effects will not actually be performed.
func New(bucketURL, identity string, dryRun bool) (*Bucket, error) {
	if bucketURL == "" {
		return nil, fmt.Errorf("empty Bucket URL")
	}
	if !strings.HasPrefix(bucketURL, "s3://") && !strings.HasPrefix(bucketURL, "gs://") {
		return nil, fmt.Errorf("invalid Bucket %q with identity %q", bucketURL, identity)
	}
	if strings.HasPrefix(bucketURL, "gs://") && identity != "" {
		return nil, fmt.Errorf("workflow-manager doesn't support alternate identities (%s) for gs:// Bucket (%q)",
			identity, bucketURL)
	}

	return &Bucket{
		service:    bucketURL[0:2],
		bucketName: bucketURL[5:],
		identity:   identity,
		dryRun:     dryRun,
	}, nil
}

// ListFiles lists the files contained in Bucket
func (b *Bucket) ListFiles() ([]string, error) {
	switch b.service {
	case "s3":
		return b.listFilesS3()
	case "gs":
		return b.listFilesGS()
	default:
		return nil, fmt.Errorf("invalid storage service %q", b.service)
	}
}

// WriteTaskMarker writes a marker for a scheduled task, which is an object in
// the bucket whose key is "task-markers/${marker}". This works as a guard
// against redundant tasks because both Amazon S3 and Google Cloud Storage offer
// strong read-after-write consistency.
//
// https://aws.amazon.com/s3/consistency/
// https://cloud.google.com/storage/docs/consistency
func (b *Bucket) WriteTaskMarker(marker string) error {
	markerObject := fmt.Sprintf("task-markers/%s", marker)
	switch b.service {
	case "s3":
		return b.writeTaskMarkerS3(markerObject)
	case "gs":
		return b.writeTaskMarkerGS(markerObject)
	default:
		return fmt.Errorf("invalid storage service %q", b.service)
	}
}

func parseS3BucketName(bucketName string) (string, string, error) {
	parts := strings.SplitN(bucketName, "/", 2)
	if len(parts) != 2 {
		return "", "", fmt.Errorf("invalid S3 Bucket name %q", bucketName)
	}

	return parts[0], parts[1], nil
}

func (b *Bucket) s3Service(region string) (*s3.S3, error) {
	sess, config, err := leaws.ClientConfig(region, b.identity)
	if err != nil {
		return nil, err
	}

	return s3.New(sess, config), nil
}

func (b *Bucket) listFilesS3() ([]string, error) {
	region, bucket, err := parseS3BucketName(b.bucketName)
	if err != nil {
		return nil, err
	}

	log.Printf("listing files in s3://%s as %q", bucket, b.identity)

	svc, err := b.s3Service(region)
	if err != nil {
		return nil, err
	}

	var output []string
	var nextContinuationToken = ""
	for {
		input := &s3.ListObjectsV2Input{
			MaxKeys: aws.Int64(1000),
			Bucket:  aws.String(bucket),
		}
		if nextContinuationToken != "" {
			input.ContinuationToken = &nextContinuationToken
		}
		resp, err := svc.ListObjectsV2(input)
		if err != nil {
			return nil, fmt.Errorf("unable to list items in Bucket %q, %w", b.bucketName, err)
		}
		for _, item := range resp.Contents {
			output = append(output, *item.Key)
		}
		if !*resp.IsTruncated {
			break
		}
		nextContinuationToken = *resp.NextContinuationToken
	}
	return output, nil
}

func (b *Bucket) writeTaskMarkerS3(marker string) error {
	region, bucket, err := parseS3BucketName(b.bucketName)
	if err != nil {
		return err
	}

	log.Printf("writing task marker to s3://%s/%s as %q", bucket, marker, b.identity)

	if b.dryRun {
		log.Printf("dry run, skipping marker write")
		return nil
	}

	svc, err := b.s3Service(region)
	if err != nil {
		return err
	}
	input := &s3.PutObjectInput{
		// Doesn't matter what the file contents are, but use the task name just
		// in case S3 balks at an empty body
		Body:   aws.ReadSeekCloser(strings.NewReader(marker)),
		Bucket: aws.String(bucket),
		Key:    aws.String(marker),
	}

	// Deliberately ignore the result, we only care if the write succeeds
	if _, err := svc.PutObject(input); err != nil {
		return fmt.Errorf("storage.PutObject: %w", err)
	}

	return nil
}

func (b *Bucket) gcsClient() (*storage.Client, error) {
	// Google documentation advises against timeouts on client creation
	// https://godoc.org/cloud.google.com/go#hdr-Timeouts_and_Cancellation
	ctx := context.Background()

	if b.identity != "" {
		return nil, fmt.Errorf("workflow-manager doesn't support non-default identity %q for GS Bucket %q", b.identity, b.bucketName)
	}

	client, err := storage.NewClient(ctx)
	if err != nil {
		return nil, fmt.Errorf("storage.newClient: %w", err)
	}

	return client, nil
}

func (b *Bucket) listFilesGS() ([]string, error) {
	// This timeout has to cover potentially numerous roundtrips to the
	// paginated API for listing objects, so we use a longer timeout than usual.
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()

	client, err := b.gcsClient()
	if err != nil {
		return nil, err
	}

	bkt := client.Bucket(b.bucketName)
	query := &storage.Query{Prefix: ""}

	log.Printf("looking for ready batches in gs://%s as (ambient service account)", b.bucketName)
	var output []string
	it := bkt.Objects(ctx, query)

	// Use the paginated API to list Bucket contents, as otherwise we would only
	// get the first 1,000 objects in the Bucket.
	// https://cloud.google.com/storage/docs/json_api/v1/objects/list
	p := iterator.NewPager(it, 1000, "")
	var objects []*storage.ObjectAttrs
	for {
		// NextPage will append to the objects slice
		nextPageToken, err := p.NextPage(&objects)
		if err != nil {
			return nil, fmt.Errorf("storage.nextPage: %w", err)
		}

		if nextPageToken == "" {
			// no more data
			break
		}
	}

	for _, obj := range objects {
		output = append(output, obj.Name)
	}

	return output, nil
}

func (b *Bucket) writeTaskMarkerGS(marker string) error {
	client, err := b.gcsClient()
	if err != nil {
		return err
	}

	bkt := client.Bucket(b.bucketName)

	log.Printf("writing task marker to gs://%s/%s as (ambient service account)",
		b.bucketName, marker)

	if b.dryRun {
		log.Printf("dry run, skipping marker write")
		return nil
	}

	object := bkt.Object(marker)

	ctx, cancel := utils.ContextWithTimeout()
	defer cancel()

	writer := object.NewWriter(ctx)
	_, err = io.WriteString(writer, marker)
	if err != nil {
		writer.Close()
		return fmt.Errorf("failed to write marker to GCS: %w", err)
	}

	// If writes to GCS fail, we won't find out until we call Close, so we don't
	// defer in order to check the error
	// https://godoc.org/cloud.google.com/go/storage#Writer.Write
	if err := writer.Close(); err != nil {
		return fmt.Errorf("failed to close GCS writer: %w", err)
	}

	return nil
}
