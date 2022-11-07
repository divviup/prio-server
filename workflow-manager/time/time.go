package time

import (
	"context"
	"encoding/json"
	"fmt"
	"time"
)

// ContextWithTimeout returns a Context and CancelFunc configured with a default
// timeout value suitable for most network requests.
func ContextWithTimeout() (context.Context, context.CancelFunc) {
	return context.WithTimeout(context.Background(), 30*time.Second)
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

// AggregationIntervalFunc represents a function that can generate an
// aggregation window (as an Interval) based on a "now" timestamp.
type AggregationIntervalFunc func(now time.Time) Interval

// StandardAggregationWindow returns an aggregation interval function which
// produces the "standard" aggregation window, i.e. the most recent aggregation
// window which is at least one grace period into the past.
func StandardAggregationWindow(aggregationPeriod, gracePeriod time.Duration) AggregationIntervalFunc {
	return func(now time.Time) Interval { return AggregationInterval(now, aggregationPeriod, gracePeriod) }
}

// OverrideAggregationWindow returns an aggregation interval function which
// always produces a specific aggregation window, specifically the aggregation
// window containing the given time.
func OverrideAggregationWindow(when time.Time, aggregationPeriod time.Duration) AggregationIntervalFunc {
	return func(time.Time) Interval { return AggregationIntervalIncluding(when, aggregationPeriod) }
}

// Interval represents a half-open interval of time.
// It includes `begin` and excludes `end`.
type Interval struct {
	// Begin is the beginning of the interval, included in the interval
	Begin time.Time
	// End is the end of the interval, excluded from the interval
	End time.Time
}

// AggregationInterval calculates the interval we want to run an aggregation
// for, if any. That is whatever interval is `gracePeriod` earlier than now and
// aligned on multiples of `aggregationPeriod` (relative to the zero time).
func AggregationInterval(now time.Time, aggregationPeriod, gracePeriod time.Duration) Interval {
	return AggregationIntervalIncluding(now.Add(-gracePeriod).Add(-aggregationPeriod), aggregationPeriod)
}

// AggregationIntervalIncluding calculates an interval of aggregation, given a
// point of time inside that interval. The start of the returned interval will
// be the given point in time, truncated to the nearest multiple of the
// aggregation period (relative to the zero time).
func AggregationIntervalIncluding(when time.Time, aggregationPeriod time.Duration) Interval {
	start := when.Truncate(aggregationPeriod)
	return Interval{
		Begin: start,
		End:   start.Add(aggregationPeriod),
	}
}

func (i Interval) String() string {
	return fmt.Sprintf("%s to %s", FmtTime(i.Begin), FmtTime(i.End))
}

// TimestampPrefixes returns a list of timestamps, truncated to the hour,
// representing hours included in the Interval. For example, if the Interval
// were 2021/01/01/00/00 - 2021/01/01/06/00, this would return
//
//	[]Timestamp {
//	     "2021/01/01/00",
//	     "2021/01/01/01",
//	     "2021/01/01/02",
//	     "2021/01/01/03",
//	     "2021/01/01/04",
//	     "2021/01/01/05",
//	}
//
// If the interval were 2021/01/01/00/00 - 2021/01/01/00/30 (less than one
// hour), this would return
// []Timestamp { "2021/01/01/00" }
//
// Note that the returned list respects the half-open nature of the Interval.
func (i Interval) TimestampPrefixes() []Timestamp {
	prefixes := []Timestamp{}
	currTime := i.Begin

	for {
		if currTime.Equal(i.End) || currTime.After(i.End) {
			break
		}

		prefixes = append(prefixes, (Timestamp)(currTime))
		currTime = currTime.Add(time.Hour)
	}

	return prefixes
}

// Length returns the Duration covered by the interval
func (i Interval) Length() time.Duration {
	return i.End.Sub(i.Begin)
}

// Includes returns true if the provided Time falls within the interval
func (i *Interval) Includes(time time.Time) bool {
	// We use before twice rather than Before and After, because Before is
	// <, and After is >, but we are processing a half-open interval so we
	// need >= and <.
	return !time.Before(i.Begin) && time.Before(i.End)
}

// FmtTime returns the input time in the same style expected by facilitator/lib.rs,
// currently "%Y/%m/%d/%H/%M"
func FmtTime(t time.Time) string {
	return (*Timestamp)(&t).String()
}

// Timestamp is an alias to time.Time with a custom JSON marshaler that
// marshals the time to UTC, with minute precision, in the format
// "2006/01/02/15/04"
type Timestamp time.Time

func (t Timestamp) MarshalJSON() ([]byte, error) {
	return json.Marshal(t.String())
}

func (t *Timestamp) UnmarshalJSON(data []byte) error {
	var timeString string
	if err := json.Unmarshal(data, &timeString); err != nil {
		return err
	}

	parsedTime, err := time.Parse("2006/01/02/15/04", timeString)
	if err != nil {
		return err
	}

	*t = Timestamp(parsedTime)
	return nil
}

func (t *Timestamp) stringWithFormat(format string) string {
	asTime := (*time.Time)(t)
	return asTime.Format(format)
}

func (t *Timestamp) String() string {
	return t.stringWithFormat("2006/01/02/15/04")
}

// TruncatedTimestamp returns the timestamp's string representation, truncated
// to the hour, with a trailing /
func (t *Timestamp) TruncatedTimestamp() string {
	return t.stringWithFormat("2006/01/02/15/")
}

// MarkerString returns the representation of the timestamp as it should be
// incorporated into a task marker
func (t *Timestamp) MarkerString() string {
	return t.stringWithFormat("2006-01-02-15-04")
}

// TruncatedMarkerString returns the representation of the timestamp as it
// should be incorporated into a task marker, truncated to the hour, with a
// trailing -
func (t *Timestamp) TruncatedMarkerString() string {
	return t.stringWithFormat("2006-01-02-15-")
}
