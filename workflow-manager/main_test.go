package main

import (
	"github.com/letsencrypt/prio-server/workflow-manager/batchpath"
	"testing"
)

func TestIntakeJobNameForBatchPath(t *testing.T) {
	var testCases = []struct {
		name     string
		input    string
		expected string
	}{
		{
			name:     "short-aggregation-name",
			input:    "kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771",
			expected: "i-kittens-seen-b8a5579af984460a-2020-10-31-20-29",
		},
		{
			name:     "long-aggregation-name",
			input:    "a-very-long-aggregation-name-that-will-get-truncated/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771",
			expected: "i-a-very-long-aggregation-nam-b8a5579af984460a-2020-10-31-20-29",
		},
	}
	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			batchPath, err := batchpath.New(testCase.input)
			if err != nil {
				t.Fatalf("unexpected batch path parse failure: %s", err)
			}

			jobName := intakeJobNameForBatchPath(batchPath)
			if jobName != testCase.expected {
				t.Errorf("expected %q, encountered %q", testCase.expected, jobName)
			}

			if len(jobName) > 63 {
				t.Errorf("job name is too long")
			}
		})
	}
}

func TestAggregationJobNameFragment(t *testing.T) {
	input := "FooBar%012345678901234567890123456789"
	id := aggregationJobNameFragment(input, 30)
	expected := "foobar-01234567890123456789012"
	if id != expected {
		t.Errorf("expected id %q, got %q", expected, id)
	}
}
