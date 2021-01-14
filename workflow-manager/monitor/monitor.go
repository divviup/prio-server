package monitor

type GaugeMonitor interface {
	Inc()
	Set(float64)
	SetToCurrentTime()
}

type NoopGauge struct {
	counted int
}

func (c *NoopGauge) Inc() {
	c.counted = c.counted + 1
}

func (c *NoopGauge) Set(value float64) {}

func (c *NoopGauge) SetToCurrentTime() {}
