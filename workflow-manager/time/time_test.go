package time

import (
	"reflect"
	"testing"
	"time"
)

func TestTimestampPrefixes(t *testing.T) {
	var testCases = []struct {
		name     string
		input    Interval
		expected []Timestamp
	}{
		{
			name: "empty interval",
			input: Interval{
				Begin: time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC),
				End:   time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC),
			},
			expected: []Timestamp{},
		},
		{
			name: "backward interval",
			input: Interval{
				Begin: time.Date(2010, 01, 01, 6, 0, 0, 0, time.UTC),
				End:   time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC),
			},
			expected: []Timestamp{},
		},
		{
			name: "multiple hours",
			input: Interval{
				Begin: time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC),
				End:   time.Date(2010, 01, 01, 6, 0, 0, 0, time.UTC),
			},
			expected: []Timestamp{
				Timestamp(time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 1, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 2, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 3, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 4, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 5, 0, 0, 0, time.UTC)),
			},
		},
		{
			name: "less than one hour",
			input: Interval{
				Begin: time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC),
				End:   time.Date(2010, 01, 01, 0, 30, 0, 0, time.UTC),
			},
			expected: []Timestamp{
				Timestamp(time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC)),
			},
		},
		{
			name: "fractional hours",
			input: Interval{
				Begin: time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC),
				End:   time.Date(2010, 01, 01, 6, 30, 0, 0, time.UTC),
			},
			expected: []Timestamp{
				Timestamp(time.Date(2010, 01, 01, 0, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 1, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 2, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 3, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 4, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 5, 0, 0, 0, time.UTC)),
				Timestamp(time.Date(2010, 01, 01, 6, 0, 0, 0, time.UTC)),
			},
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			prefixes := testCase.input.TimestampPrefixes()
			if !reflect.DeepEqual(prefixes, testCase.expected) {
				t.Errorf("expected prefixes %v, got %v", testCase.expected, prefixes)
			}
		})
	}
}

func TestIntervalIncludes(t *testing.T) {
	interval := Interval{
		Begin: time.Date(2010, 1, 1, 0, 0, 0, 0, time.UTC),
		End:   time.Date(2010, 1, 1, 0, 1, 0, 0, time.UTC),
	}

	var testCases = []struct {
		name     string
		input    time.Time
		expected bool
	}{
		{
			name:     "before interval",
			input:    time.Date(2009, 1, 1, 0, 0, 0, 0, time.UTC),
			expected: false,
		},
		{
			name:     "at interval start",
			input:    time.Date(2010, 1, 1, 0, 0, 0, 0, time.UTC),
			expected: true,
		},
		{
			name:     "within interval",
			input:    time.Date(2010, 1, 1, 0, 0, 1, 0, time.UTC),
			expected: true,
		},
		{
			name:     "at interval end",
			input:    time.Date(2010, 1, 1, 0, 1, 0, 0, time.UTC),
			expected: false,
		},
		{
			name:     "past interval",
			input:    time.Date(2010, 1, 1, 0, 2, 0, 0, time.UTC),
			expected: false,
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			if interval.Includes(testCase.input) != testCase.expected {
				t.Errorf("unexpected result for test case %q", testCase.name)
			}
		})
	}
}
