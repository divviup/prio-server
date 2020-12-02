package retry

import (
	"fmt"
	"log"
	"time"
)

// Retry defines a behavior for Retryable functions
type Retry struct {
	Identifier string
	Retryable
	ShouldRequeue
	MaxTries int

	// TimeBetweenTries will block the executing goroutine until that amount of time has passed
	// If you are using this, make sure you're executing Retry on a new goroutine to not block
	// your code
	TimeBetweenTries time.Duration
}

// Retryable is a function that takes no arguments and returns only an error
type Retryable func() error

// ShouldRequeue takes a single argument of an error and returns a true or false.
// This function is responsible to decide if Retry should re-attempt executing Retryable
type ShouldRequeue func(err error) bool

// Start starts attempting the function
func (r *Retry) Start() error {
	var lastError error

	for i := 0; i < r.MaxTries; i++ {
		lastError = r.Retryable()
		if lastError == nil {
			return nil
		}

		if !r.ShouldRequeue(lastError) {
			return lastError
		}

		log.Printf("Requeuing %s", r.Identifier)
		if r.TimeBetweenTries > 0 {
			time.Sleep(r.TimeBetweenTries)
		}
	}

	// If it reached here the errors continued...
	return fmt.Errorf("%s kept failing after retries - last error: %s", r.Identifier, lastError)
}
