package bucket

import (
	"context"
	"fmt"
	"log"
	"strings"

	"cloud.google.com/go/storage"
	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/arn"
	"github.com/aws/aws-sdk-go/aws/credentials"
	"github.com/aws/aws-sdk-go/aws/credentials/stscreds"
	aws_session "github.com/aws/aws-sdk-go/aws/session"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/aws/aws-sdk-go/service/sts"
	"github.com/letsencrypt/prio-server/workflow-manager/tokenfetcher"
	"google.golang.org/api/iterator"
)

// Bucket represents a general bucket of data
type Bucket struct {
	// service is either "s3", or "gs"
	service string
	// bucketName includes the region for S3
	bucketName string
	identity   string
}

// New creates a new Bucket from a URL and identity
func New(bucketURL, identity string) (*Bucket, error) {
	if bucketURL == "" {
		log.Fatalf("empty Bucket URL")
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
	}, nil
}

// ListFiles lists the files contained in Bucket
func (b *Bucket) ListFiles(ctx context.Context) ([]string, error) {
	switch b.service {
	case "s3":
		return b.listFilesS3(ctx)
	case "gs":
		return b.listFilesGS(ctx)
	default:
		return nil, fmt.Errorf("invalid storage service %q", b.service)
	}
}

func webIDP(sess *aws_session.Session, identity string) (*credentials.Credentials, error) {
	parsed, err := arn.Parse(identity)
	if err != nil {
		return nil, err
	}
	audience := fmt.Sprintf("sts.amazonaws.com/%s", parsed.AccountID)

	stsSTS := sts.New(sess)
	roleSessionName := ""
	roleProvider := stscreds.NewWebIdentityRoleProviderWithToken(
		stsSTS, identity, roleSessionName, tokenfetcher.NewTokenFetcher(audience))

	return credentials.NewCredentials(roleProvider), nil
}

func (b *Bucket) listFilesS3(ctx context.Context) ([]string, error) {
	parts := strings.SplitN(b.bucketName, "/", 2)
	if len(parts) != 2 {
		return nil, fmt.Errorf("invalid S3 Bucket name %q", b.bucketName)
	}
	region := parts[0]
	bucket := parts[1]
	sess, err := aws_session.NewSession()
	if err != nil {
		return nil, fmt.Errorf("making AWS session: %w", err)
	}

	log.Printf("listing files in s3://%s as %q", bucket, b.identity)
	config := aws.NewConfig().
		WithRegion(region)
	if b.identity != "" {
		creds, err := webIDP(sess, b.identity)
		if err != nil {
			return nil, err
		}
		config = config.WithCredentials(creds)
	}
	svc := s3.New(sess, config)
	var output []string
	var nextContinuationToken = ""
	for {
		input := &s3.ListObjectsV2Input{
			// We choose a lower number than the default max of 1000 to ensure we exercise the
			// cursoring case regularly.
			MaxKeys: aws.Int64(100),
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

func (b *Bucket) listFilesGS(ctx context.Context) ([]string, error) {
	if b.identity != "" {
		return nil, fmt.Errorf("workflow-manager doesn't support non-default identity %q for GS Bucket %q", b.identity, b.bucketName)
	}
	client, err := storage.NewClient(ctx)
	if err != nil {
		return nil, fmt.Errorf("storage.newClient: %w", err)
	}

	bkt := client.Bucket(b.bucketName)
	query := &storage.Query{Prefix: ""}

	log.Printf("looking for ready batches in gs://%s as (ambient service account)", b.bucketName)
	var output []string
	it := bkt.Objects(ctx, query)

	// Use the paginated API to list Bucket contents, as otherwise we would only get the first 1,000 objects in the Bucket.
	// As in the S3 case above, we get 100 results at a time to ensure we exercise the pagination case.
	// https://cloud.google.com/storage/docs/json_api/v1/objects/list
	p := iterator.NewPager(it, 100, "")
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
