package utils

import (
	"time"
)

// Index returns the appropriate int to use in construction of filenames
// based on whether the file creator was the "first" aka PHA server.
func Index(isFirst bool) int {
	if isFirst {
		return 0
	}
	return 1
}

// Clock allows mocking of time for testing purposes
type Clock struct {
	now time.Time
}

// DefaultClock reutrns a Clock that returns the real time from time.Now
func DefaultClock() Clock {
	return Clock{}
}

// ClockWithFixedNow returns a Clock that always returns the provided now
func ClockWithFixedNow(now time.Time) Clock {
	return Clock{now: now}
}

// Now returns the current time according to this clock
func (c *Clock) Now() time.Time {
	if c.now.IsZero() {
		return time.Now()
	}
	return c.now
}
