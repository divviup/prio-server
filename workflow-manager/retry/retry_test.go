package retry

import (
	"fmt"
	"strings"
	"testing"
)

func TestRetryExactTimes(t *testing.T) {
	sharedMemory := make([]string, 0)

	r := Retry{
		Identifier: "Retryable",
		Retryable: func() error {
			if len(sharedMemory) != 5 {
				sharedMemory = append(sharedMemory, "Some Value")
				return fmt.Errorf("testerror")
			}

			return nil
		},
		ShouldRequeue: func(err error) bool {
			return strings.Contains(err.Error(), "testerror")
		},
		MaxTries:         10,
		TimeBetweenTries: 0,
	}

	err := r.Start()

	if len(sharedMemory) != 5 {
		t.Errorf("Shared memory should've contained 5 elements")
	}

	if err != nil {
		t.Errorf("Final error should've been nil")
	}
}

func TestShouldFail(t *testing.T) {
	r := Retry{
		Identifier: "Retryable",
		Retryable: func() error {
			return fmt.Errorf("testerror")
		},
		ShouldRequeue: func(err error) bool {
			return strings.Contains(err.Error(), "testerror")
		},
		MaxTries:         5,
		TimeBetweenTries: 0,
	}

	err := r.Start()

	if err == nil {
		t.Errorf("Final error should not have been nil")
	}
}
