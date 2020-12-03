package monitor

import "testing"

func TestNoopCounterIncrement(t *testing.T) {
	c := NoopCounter{}

	c.Inc()
	c.Inc()

	if c.counted != 2 {
		t.Error("Should have been counted twice")
	}
}
