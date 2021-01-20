package monitor

import "testing"

func TestNoopGaugeIncrement(t *testing.T) {
	c := NoopGauge{}

	c.Inc()
	c.Inc()

	if c.counted != 2 {
		t.Error("Should have been counted twice")
	}
}
