package storage

import (
	"context"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/rs/zerolog/log"

	leaws "github.com/letsencrypt/prio-server/workflow-manager/aws"
	"github.com/letsencrypt/prio-server/workflow-manager/batchpath"
	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"

	"cloud.google.com/go/storage"
	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/aws/aws-sdk-go/service/s3/s3iface"
	"google.golang.org/api/iterator"
)

const (
	taskMarkerDirectory = "task-markers"
)

// Bucket represents a cloud storage bucket
type Bucket interface {
	// ListAggregationIDs returns a list of aggregation IDs present in the
	// bucket, but without enumerating every object in the bucket.
	ListAggregationIDs() ([]string, error)
	// ListBatchFiles returns a list of objects in this bucket that are part of
	// a batch (e.g., ingestion or validation) whose timestamp is within the
	// provided interval.
	ListBatchFiles(aggregationID string, interval wftime.Interval) ([]string, error)
	// ListIntakeTaskMarkers returns a list of objects in this storage that are
	// intake task markers for batches whose timestamp is within the provided
	// interval.
	ListIntakeTaskMarkers(aggregationID string, interval wftime.Interval) ([]string, error)
	// ListAggregateTaskMarkers lists all markers for aggregation tasks for the
	// specified aggregation ID. Does not take an interval, on the premise that
	// aggregation tasks are infrequent enough that listing all the markers in
	// the bucket will be cheap. For instance, in production, we currently run
	// aggregations every eight hours and we retain seven days' worth of data in
	// storage buckets, meaning this query should return 3 x 7 = 21 objects.
	ListAggregateTaskMarkers(aggregationID string) ([]string, error)
	// WriteTaskMarker writes a marker for a scheduled task, which is an object in
	// the bucket whose key is "task-markers/${marker}". This works as a guard
	// against redundant tasks because both Amazon S3 and Google Cloud Storage offer
	// strong read-after-write consistency.
	//
	// https://aws.amazon.com/s3/consistency/
	// https://cloud.google.com/storage/docs/consistency
	WriteTaskMarker(marker string) error
}

// NewBucket creates a new Bucket from a URL and identity. If dryRun is true,
// then any operations with side effects will not actually be performed.
// bucketURL must have a scheme indicating which cloud storage service should be
// used (e.g., "gs://" for Google Cloud Storage or "s3://" for Amazon S3).
func NewBucket(bucketURL, identity string, dryRun bool) (Bucket, error) {
	if bucketURL == "" {
		return nil, fmt.Errorf("empty Bucket URL")
	}

	if len(bucketURL) < 4 {
		return nil, fmt.Errorf("bucket URL too short to contain scheme: %q", bucketURL)
	}

	service := bucketURL[0:2]
	bucketName := bucketURL[5:]

	switch service {
	case "s3":
		return newS3(bucketName, identity, dryRun)
	case "gs":
		if identity != "" {
			return nil, fmt.Errorf("workflow-manager doesn't support alternate identities (%s) for gs:// Bucket (%q)",
				identity, bucketName)
		}
		return newGCS(bucketName, dryRun)
	default:
		return nil, fmt.Errorf("bucket URL has unrecognized scheme: %q", bucketURL)
	}
}

func taskMarkerObject(task string) string {
	return fmt.Sprintf("%s/%s", taskMarkerDirectory, task)
}

// filterTaskMarkers takes a list of directories (i.e., the top level of a
// storage bucket's contents) and returns the list of aggregations in the bucket
func filterTaskMarkers(directories []string) []string {
	var aggregationIDs []string
	for _, aggregationID := range directories {
		// "task-markers" is a reserved name and cannot be an aggregation
		if aggregationID == taskMarkerDirectory {
			continue
		}
		aggregationIDs = append(aggregationIDs, aggregationID)
	}

	return aggregationIDs
}

type listResult struct {
	prefixes []string
	objects  []string
}

// S3Bucket represents an AWS S3 bucket
type S3Bucket struct {
	// region is the AWS region the bucket is in. e.g., "us-west-1".
	region string
	// bucketName is the name of the bucket, without any service prefix
	bucketName string
	// identity is the ARN of an AWS entity that should be assumed to access the
	// bucket
	identity string
	// dryRun controls whether any operations are actually performed by this
	// S3Bucket.
	dryRun bool
	// s3Service is an implementation of s3iface.S3API that may be optionally
	// provided. If set, it will be used for all S3 API calls. If unset,
	// S3Bucket will use the AWS SDK to create a client that uses the real S3.
	s3Service s3iface.S3API
}

func newS3(bucketName, identity string, dryRun bool) (*S3Bucket, error) {
	// bucket name should be "<region>/<name>", e.g., "us-west-1/my-cool-bucket"
	parts := strings.SplitN(bucketName, "/", 2)
	if len(parts) != 2 {
		return nil, fmt.Errorf("invalid S3 Bucket name %q", bucketName)
	}
	return &S3Bucket{
		region:     parts[0],
		bucketName: parts[1],
		identity:   identity,
		dryRun:     dryRun,
	}, nil
}

func (b *S3Bucket) service() (s3iface.S3API, error) {
	if b.s3Service != nil {
		return b.s3Service, nil
	}

	sess, config, err := leaws.ClientConfig(b.region, b.identity)
	if err != nil {
		return nil, err
	}

	b.s3Service = s3.New(sess, config)
	return b.s3Service, nil
}

func (b *S3Bucket) ListAggregationIDs() ([]string, error) {
	// To list the top level "directories" in an S3 bucket, we set no prefix and
	// delimiter = "/". There's no particularly good documentation on how
	// delimiter and prefix behave in the ListObjectsV2 API (but see [1], [2])
	// but empirically this combination works.
	// [1] https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListObjectsV2.html
	// [2] https://docs.aws.amazon.com/AmazonS3/latest/dev/ListingKeysHierarchy.html
	listResult, err := b.listObjects("", s3.ListObjectsV2Input{
		Delimiter: aws.String("/"),
	})
	if err != nil {
		return nil, err
	}

	directories := []string{}
	for _, result := range listResult.prefixes {
		directories = append(directories, strings.TrimSuffix(result, "/"))
	}

	return filterTaskMarkers(directories), nil
}

func (b *S3Bucket) ListBatchFiles(aggregationID string, interval wftime.Interval) ([]string, error) {
	// S3's API does not let us express a lexicographical range of keys like GCS
	// does, so we have to make do with the prefix parameter. We break the
	// interval into hour long chunks and make a ListObjectsV2 request for each
	// to only list those objects whose timestamps fall within the interval.
	// This assumes that an hour is the appropriate atom of time on which to
	// divide requests. We could also have figured out the next prefix up that
	// captures the entire interval and list all objects with that prefix (e.g.,
	// for the interval 2010/01/01/00/00 - 2010/01/01/06/00, list everything
	// with prefix aggregation-id/2010/01/01/), and only do a single
	// ListObjectsV2 request but:
	// 	(1) that is harder to implement
	//	(2) ListObjectsV2 is paginated, which means we will be making
	//		requests to S3 in proportion to the total number of objects,
	//		regardless of how many ListObjectsV2 calls we make.
	// We could also dynamically determine if an atom smaller than 1 hour is
	// appropriate (e.g., return prefixes describing minutes for a 10 minute
	// interval). In practice, the intake-max-age and aggregation-period values
	// used by workflow-manager are almost always multiples of hours, and if a
	// caller wishes to use a narrower Interval, or rather any Interval that is
	// not made of whole hours, we query some extra data from S3 and send them
	// down the slow path of filtering the S3 results through
	// batchpath.List.WithinInterval().
	objects := []string{}
	for _, timestampPrefix := range interval.TimestampPrefixes() {
		listResult, err := b.listObjects("", s3.ListObjectsV2Input{
			Prefix: aws.String(fmt.Sprintf("%s/%s", aggregationID, timestampPrefix.TruncatedTimestamp())),
		})
		if err != nil {
			return nil, err
		}
		objects = append(objects, listResult.objects...)
	}

	if interval.Length().Truncate(time.Hour) < interval.Length() {
		// slow path: the interval is not an integer number of hours, so we must
		// discard extraneous results that do not fall within the interval
		batchPaths, err := batchpath.NewList(objects)
		if err != nil {
			return nil, err
		}

		return batchPaths.WithinInterval(interval), nil
	}

	return objects, nil
}

func (b *S3Bucket) ListIntakeTaskMarkers(aggregationID string, interval wftime.Interval) ([]string, error) {
	// See the comment in ListBatchFiles for discussion of the usage of
	// interval.TimestampPrefixes. The difference here is that we don't bother
	// discarding extraneous results that fall outside of the provided interval:
	// there's no harm if the returned list of task markers includes tasks that
	// fall outside the interval.
	objects := []string{}
	for _, timestampPrefix := range interval.TimestampPrefixes() {
		prefix := fmt.Sprintf("%s/intake-%s-%s", taskMarkerDirectory, aggregationID, timestampPrefix.TruncatedMarkerString())
		listResult, err := b.listObjects(taskMarkerDirectory+"/", s3.ListObjectsV2Input{
			Prefix: aws.String(prefix),
		})
		if err != nil {
			return nil, err
		}
		objects = append(objects, listResult.objects...)
	}

	return objects, nil
}

func (b *S3Bucket) ListAggregateTaskMarkers(aggregationID string) ([]string, error) {
	prefix := fmt.Sprintf("%s/aggregate-%s-", taskMarkerDirectory, aggregationID)
	listResult, err := b.listObjects(taskMarkerDirectory+"/", s3.ListObjectsV2Input{
		Prefix: aws.String(prefix),
	})
	if err != nil {
		return nil, err
	}

	return listResult.objects, nil
}

func (b *S3Bucket) listObjects(trimObjectPrefix string, listInput s3.ListObjectsV2Input) (*listResult, error) {
	log.Debug().Msgf("listing files in s3://%s as %q", b.bucketName, b.identity)

	svc, err := b.service()
	if err != nil {
		return nil, err
	}

	var output listResult
	var nextContinuationToken = ""
	for {
		listInput.MaxKeys = aws.Int64(1000)
		listInput.Bucket = aws.String(b.bucketName)
		if nextContinuationToken != "" {
			listInput.ContinuationToken = &nextContinuationToken
		}
		resp, err := svc.ListObjectsV2(&listInput)
		if err != nil {
			return nil, fmt.Errorf("unable to list items in Bucket %q, %w", b.bucketName, err)
		}
		for _, item := range resp.Contents {
			trimmedObjectKey := strings.TrimPrefix(*item.Key, trimObjectPrefix)
			output.objects = append(output.objects, trimmedObjectKey)
		}
		for _, item := range resp.CommonPrefixes {
			output.prefixes = append(output.prefixes, *item.Prefix)
		}
		if !*resp.IsTruncated {
			break
		}
		nextContinuationToken = *resp.NextContinuationToken
	}
	return &output, nil
}

func (b *S3Bucket) WriteTaskMarker(marker string) error {
	markerObject := taskMarkerObject(marker)
	log.Info().Msgf("writing task marker to s3://%s/%s as %q", b.bucketName, markerObject, b.identity)

	if b.dryRun {
		log.Info().Msg("dry run, skipping marker write")
		return nil
	}

	svc, err := b.service()
	if err != nil {
		return err
	}
	input := &s3.PutObjectInput{
		// Doesn't matter what the file contents are, but use the task name just
		// in case S3 balks at an empty body
		Body:   aws.ReadSeekCloser(strings.NewReader(marker)),
		Bucket: aws.String(b.bucketName),
		Key:    aws.String(markerObject),
	}

	// Deliberately ignore the result, we only care if the write succeeds
	if _, err := svc.PutObject(input); err != nil {
		return fmt.Errorf("storage.PutObject: %w", err)
	}

	return nil
}

type GCSBucket struct {
	// bucketName is the name of the bucket, without any service prefix
	bucketName string
	dryRun     bool
}

func newGCS(bucketName string, dryRun bool) (*GCSBucket, error) {
	return &GCSBucket{
		bucketName: bucketName,
		dryRun:     dryRun,
	}, nil
}

func (b *GCSBucket) client() (*storage.Client, error) {
	// Google documentation advises against timeouts on client creation
	// https://godoc.org/cloud.google.com/go#hdr-Timeouts_and_Cancellation
	ctx := context.Background()

	client, err := storage.NewClient(ctx)
	if err != nil {
		return nil, fmt.Errorf("storage.newClient: %w", err)
	}

	return client, nil
}

func (b *GCSBucket) ListAggregationIDs() ([]string, error) {
	// We want to list the top level "directories" in the bucket to discover
	// what aggregations are present, so set no prefix and the "/" delimiter to
	// get a listing of top-level "directories" in the bucket. For discussion of
	// delimiter and prefix parameters:
	// https://cloud.google.com/storage/docs/json_api/v1/objects/list
	listResult, err := b.listObjects("", storage.Query{
		Delimiter: "/",
	})
	if err != nil {
		return nil, err
	}

	return filterTaskMarkers(listResult.prefixes), nil
}

func (b *GCSBucket) ListBatchFiles(aggregationID string, interval wftime.Interval) ([]string, error) {
	startOffset := fmt.Sprintf("%s/%s", aggregationID, wftime.FmtTime(interval.Begin))
	endOffset := fmt.Sprintf("%s/%s", aggregationID, wftime.FmtTime(interval.End))

	listResult, err := b.listObjects("", storage.Query{
		StartOffset: startOffset,
		EndOffset:   endOffset,
	})
	if err != nil {
		return nil, err
	}

	return listResult.objects, nil
}

func (b *GCSBucket) ListIntakeTaskMarkers(aggregationID string, interval wftime.Interval) ([]string, error) {
	startOffset := fmt.Sprintf("%s/intake-%s-%s", taskMarkerDirectory, aggregationID, (*wftime.Timestamp)(&interval.Begin).MarkerString())
	endOffset := fmt.Sprintf("%s/intake-%s-%s", taskMarkerDirectory, aggregationID, (*wftime.Timestamp)(&interval.End).MarkerString())

	listResult, err := b.listObjects(taskMarkerDirectory+"/", storage.Query{
		StartOffset: startOffset,
		EndOffset:   endOffset,
	})
	if err != nil {
		return nil, err
	}

	return listResult.objects, nil
}

func (b *GCSBucket) ListAggregateTaskMarkers(aggregationID string) ([]string, error) {
	prefix := fmt.Sprintf("%s/aggregate-%s-", taskMarkerDirectory, aggregationID)
	listResult, err := b.listObjects(taskMarkerDirectory+"/", storage.Query{
		Prefix: prefix,
	})
	if err != nil {
		return nil, err
	}

	return listResult.objects, nil
}

func (b *GCSBucket) listObjects(trimObjectPrefix string, query storage.Query) (*listResult, error) {
	// This timeout has to cover potentially numerous roundtrips to the
	// paginated API for listing objects, so we use a longer timeout than usual.
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()

	client, err := b.client()
	if err != nil {
		return nil, err
	}

	bkt := client.Bucket(b.bucketName)

	// We only need the "Name" (for objects). Prefix will be set on objects in
	// the response if the query included Delimiter.
	// https://pkg.go.dev/cloud.google.com/go/storage#Query.SetAttrSelection
	if err := query.SetAttrSelection([]string{"Name"}); err != nil {
		return nil, fmt.Errorf("query.SetAttrSelection: %w", err)
	}

	log.Debug().Msgf("listing bucket gs://%s as (ambient service account)", b.bucketName)
	var output listResult
	it := bkt.Objects(ctx, &query)

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

	for _, object := range objects {
		if object.Prefix != "" {
			output.prefixes = append(output.prefixes, strings.TrimSuffix(object.Prefix, "/"))
		} else if object.Name != "" {
			trimmedName := strings.TrimPrefix(object.Name, trimObjectPrefix)
			output.objects = append(output.objects, trimmedName)
		} else {
			return nil, fmt.Errorf("object listing contained neither Prefix or Name: %v", object)
		}
	}

	return &output, nil
}

func (b *GCSBucket) WriteTaskMarker(marker string) error {
	client, err := b.client()
	if err != nil {
		return err
	}

	bkt := client.Bucket(b.bucketName)

	markerObject := taskMarkerObject(marker)
	log.Info().Msgf("writing task marker to gs://%s/%s as (ambient service account)",
		b.bucketName, markerObject)

	if b.dryRun {
		log.Info().Msg("dry run, skipping marker write")
		return nil
	}

	object := bkt.Object(markerObject)

	ctx, cancel := wftime.ContextWithTimeout()
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
