package monitor

type CounterMonitor interface {
	Inc()
}

type NoopCounter struct {
	counted int
}

func (c *NoopCounter) Inc() {
	c.counted = c.counted + 1
}
