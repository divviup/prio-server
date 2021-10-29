package storage

import (
	"reflect"
	"testing"
	"time"

	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/aws/aws-sdk-go/service/s3/s3iface"

	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"
)

func TestNewBucket(t *testing.T) {
	var testCases = []struct {
		name              string
		bucketURL         string
		identity          string
		expectedS3Bucket  *S3Bucket
		expectedGCSBucket *GCSBucket
		expectedError     bool
	}{
		{
			name:          "empty URL",
			bucketURL:     "",
			expectedError: true,
		},
		{
			name:          "no scheme",
			bucketURL:     "noscheme",
			expectedError: true,
		},
		{
			name:          "unknown service",
			bucketURL:     "qq://somebucket",
			expectedError: true,
		},
		{
			name:          "s3 only scheme",
			bucketURL:     "s3://",
			expectedError: true,
		},
		{
			name:          "s3 no region",
			bucketURL:     "s3://bucketname",
			expectedError: true,
		},
		{
			name:      "s3 OK",
			bucketURL: "s3://region/bucketname",
			identity:  "somebody",
			expectedS3Bucket: &S3Bucket{
				region:     "region",
				bucketName: "bucketname",
				identity:   "somebody",
				dryRun:     false,
			},
		},
		{
			name:          "gs has identity",
			bucketURL:     "gs://bucketname",
			identity:      "not-empty-string",
			expectedError: true,
		},
		{
			name:      "gs OK",
			bucketURL: "gs://bucketname",
			expectedGCSBucket: &GCSBucket{
				bucketName: "bucketname",
				dryRun:     false,
			},
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			bucket, err := NewBucket(testCase.bucketURL, testCase.identity, false)
			if testCase.expectedS3Bucket != nil {
				if err != nil {
					t.Errorf("unexpected error %q", err)
				}
				s3Bucket, ok := bucket.(*S3Bucket)
				if !ok {
					t.Errorf("bucket is not S3Bucket: %q (%T)", bucket, bucket)
				}
				if testCase.expectedS3Bucket.bucketName != s3Bucket.bucketName ||
					testCase.expectedS3Bucket.region != s3Bucket.region ||
					testCase.expectedS3Bucket.identity != s3Bucket.identity ||
					testCase.expectedS3Bucket.dryRun != s3Bucket.dryRun {
					t.Errorf("wrong S3 bucket: %v", s3Bucket)
				}
			}
			if testCase.expectedGCSBucket != nil {
				if err != nil {
					t.Errorf("unexpected error %q", err)
				}
				gcsBucket, ok := bucket.(*GCSBucket)
				if !ok {
					t.Errorf("bucket is not GCSBucket: %q (%T)", bucket, bucket)
				}
				if testCase.expectedGCSBucket.bucketName != gcsBucket.bucketName ||
					testCase.expectedGCSBucket.dryRun != gcsBucket.dryRun {
					t.Errorf("wrong GCS bucket: %q", bucket)
				}
			}
			if testCase.expectedError && err == nil {
				t.Errorf("expected error, got bucket %q", bucket)
			}
		})
	}
}

type mockS3Service struct {
	s3iface.S3API
	listOutputs       []s3.ListObjectsV2Output
	listOutputCounter int
}

func (m *mockS3Service) ListObjectsV2(*s3.ListObjectsV2Input) (*s3.ListObjectsV2Output, error) {
	m.listOutputCounter += 1
	return &m.listOutputs[m.listOutputCounter-1], nil
}

func (m *mockS3Service) PutObject(*s3.PutObjectInput) (*s3.PutObjectOutput, error) {
	return nil, nil
}

func TestS3ClientListAggregationIDs(t *testing.T) {
	mockS3Service := mockS3Service{
		listOutputs: []s3.ListObjectsV2Output{
			{
				CommonPrefixes: []*s3.CommonPrefix{
					{Prefix: aws.String("aggregation-id-1/")},
					{Prefix: aws.String("aggregation-id-2/")},
					{Prefix: aws.String("task-markers/")},
				},
				IsTruncated: aws.Bool(false),
			},
		},
	}

	s3Bucket, err := newS3("region/bucketname", "", false)
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	s3Bucket.s3Service = &mockS3Service

	aggregationIDs, err := s3Bucket.ListAggregationIDs()
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	if !reflect.DeepEqual(aggregationIDs, []string{"aggregation-id-1", "aggregation-id-2"}) {
		t.Errorf("unexpected aggregation ID %q", aggregationIDs)
	}
}

func TestS3ClientListBatchFiles(t *testing.T) {
	intervalStart, _ := time.Parse("2006/01/02/15/04", "2020/10/31/20/00")
	intervalThreeHours, _ := time.Parse("2006/01/02/15/04", "2020/10/31/23/00")
	intervalTwoAndAHalfHours, _ := time.Parse("2006/01/02/15/04", "2020/10/31/22/30")

	mockS3Service := mockS3Service{
		listOutputs: []s3.ListObjectsV2Output{
			// Interval is either 3h or 2.5h, which should yield three requests
			{
				Contents: []*s3.Object{
					{Key: aws.String("kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch")},
					{Key: aws.String("kittens-seen/2020/10/31/20/35/0f0317b2-c612-48c2-b08d-d98529d6eae4.batch")},
				},
				IsTruncated: aws.Bool(false),
			},
			{
				Contents: []*s3.Object{
					{Key: aws.String("kittens-seen/2020/10/31/21/29/7a1c0fbc-2b7f-4307-8185-9ea88961bb64.batch")},
					{Key: aws.String("kittens-seen/2020/10/31/21/35/af97ffdd-00fc-4d6a-9790-e5c0de82e7b0.batch")},
				},
				IsTruncated: aws.Bool(false),
			},
			{
				Contents: []*s3.Object{
					{Key: aws.String("kittens-seen/2020/10/31/22/29/dc1dcb80-25a7-4e3f-9ff5-552b7d69e21a.batch")},
					// This last batch is *after* the end of
					// intervalTwoAndAHalfHours but *within* intervalThreeHours
					{Key: aws.String("kittens-seen/2020/10/31/22/35/79f0a477-b65c-47c9-a2bf-a3b56c33824a.batch")},
				},
				IsTruncated: aws.Bool(false),
			},
		},
	}

	expectedBatchFiles := []string{
		"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch",
		"kittens-seen/2020/10/31/20/35/0f0317b2-c612-48c2-b08d-d98529d6eae4.batch",
		"kittens-seen/2020/10/31/21/29/7a1c0fbc-2b7f-4307-8185-9ea88961bb64.batch",
		"kittens-seen/2020/10/31/21/35/af97ffdd-00fc-4d6a-9790-e5c0de82e7b0.batch",
		"kittens-seen/2020/10/31/22/29/dc1dcb80-25a7-4e3f-9ff5-552b7d69e21a.batch",
		"kittens-seen/2020/10/31/22/35/79f0a477-b65c-47c9-a2bf-a3b56c33824a.batch",
	}

	s3Bucket, err := newS3("region/bucketname", "", false)
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	s3Bucket.s3Service = &mockS3Service

	batchFiles, err := s3Bucket.ListBatchFiles("kittens-seen", wftime.Interval{
		Begin: intervalStart,
		End:   intervalThreeHours,
	})
	if err != nil {
		t.Errorf("unexpected error %q", err)
	}
	if !reflect.DeepEqual(expectedBatchFiles, batchFiles) {
		t.Errorf("unexpected batch files %q", batchFiles)
	}
	if mockS3Service.listOutputCounter != 3 {
		t.Errorf("unexpected number of ListObjectV2 requests %d", mockS3Service.listOutputCounter)
	}

	// Reset the mockS3Service so we can use it again
	mockS3Service.listOutputCounter = 0
	batchFiles, err = s3Bucket.ListBatchFiles("kittens-seen", wftime.Interval{
		Begin: intervalStart,
		End:   intervalTwoAndAHalfHours,
	})
	if err != nil {
		t.Errorf("unexpected error %q", err)
	}
	// We expect the last result from ListObjectsV2 to be discarded because it
	// falls outside the interval
	if !reflect.DeepEqual(expectedBatchFiles[:5], batchFiles) {
		t.Errorf("unexpected batchfiles %q", batchFiles)
	}
	if mockS3Service.listOutputCounter != 3 {
		t.Errorf("unexpected number of ListObjectV2 requests %d", mockS3Service.listOutputCounter)
	}
}

func TestS3ListIntakeTaskMarkers(t *testing.T) {
	intervalStart, _ := time.Parse("2006/01/02/15/04", "2020/10/31/20/00")
	intervalEnd, _ := time.Parse("2006/01/02/15/04", "2020/10/31/21/00")

	mockS3Service := mockS3Service{
		listOutputs: []s3.ListObjectsV2Output{
			{
				Contents: []*s3.Object{
					{Key: aws.String("task-markers/intake-kittens-seen-1")},
					{Key: aws.String("task-markers/intake-kittens-seen-2")},
					{Key: aws.String("task-markers/intake-kittens-seen-3")},
				},
				IsTruncated: aws.Bool(false),
			},
		},
	}

	s3Bucket, err := newS3("region/bucketname", "", false)
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	s3Bucket.s3Service = &mockS3Service

	markers, err := s3Bucket.ListIntakeTaskMarkers("kittens-seen", wftime.Interval{
		Begin: intervalStart,
		End:   intervalEnd,
	})
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	if !reflect.DeepEqual(markers, []string{
		"intake-kittens-seen-1",
		"intake-kittens-seen-2",
		"intake-kittens-seen-3",
	}) {
		t.Errorf("unexpected intake markers %q", markers)
	}
}

func TestS3ListAggregateTaskMarkers(t *testing.T) {
	mockS3Service := mockS3Service{
		listOutputs: []s3.ListObjectsV2Output{
			{
				Contents: []*s3.Object{
					{Key: aws.String("task-markers/aggregate-kittens-seen-1")},
					{Key: aws.String("task-markers/aggregate-kittens-seen-2")},
					{Key: aws.String("task-markers/aggregate-kittens-seen-3")},
				},
				IsTruncated: aws.Bool(false),
			},
		},
	}

	s3Bucket, err := newS3("region/bucketname", "", false)
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	s3Bucket.s3Service = &mockS3Service

	markers, err := s3Bucket.ListAggregateTaskMarkers("kittens-seen")
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	if !reflect.DeepEqual(markers, []string{
		"aggregate-kittens-seen-1",
		"aggregate-kittens-seen-2",
		"aggregate-kittens-seen-3",
	}) {
		t.Errorf("unexpected aggregate markers %q", markers)
	}
}
