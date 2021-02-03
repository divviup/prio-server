package limiter

import "sync"

type token int32

// Limiter manages a pool of tokens, each of which represent a job, and ensures
// that only a fixed number of jobs are executing concurrently.
type Limiter struct {
	tokens chan token
}

// Ticket represents a current executing job.
type Ticket struct {
	token
	lock  sync.Mutex
	dirty bool
}

// New creates a new instance of Limiter that limits to maxWorkers number of
// concurrently running tasks
func New(maxWorkers int32) *Limiter {
	// We use a buffered channel of maxWorkers capacity and fill that with
	// maxWorkers tokens. Before running, a task must retrieve a token from the
	// channel and once complete, write it back to the channel. Since there are
	// only maxWorkers tokens in the channel, any task trying to start while
	// maxWorkers tasks are already executing will be blocked until the previous
	// tasks release their tokens.
	l := &Limiter{make(chan token, maxWorkers)}

	// These values can be anything, we just use this loop to make them
	for i := int32(0); i < maxWorkers; i++ {
		l.tokens <- token(i)
	}

	return l
}

// Execute blocks until a token is available and then synchronously executes the
// provided job. The caller is responsible for returning the token to the
// limiter by calling limiter.Done(ticket) with the Ticket passed into job when
// the job is finished, which might be after job itself returns if the job
// creates any goroutines.
func (l *Limiter) Execute(job func(ticket *Ticket)) {
	token := <-l.tokens
	ticket := &Ticket{
		token: token,
		lock:  sync.Mutex{},
		dirty: false,
	}
	job(ticket)
}

// Done signals that the execution for this particular job is complete.
func (l *Limiter) Done(ticket *Ticket) {
	ticket.lock.Lock()
	defer ticket.lock.Unlock()
	// Ticket was already used, don't try to put it back into the channels
	if ticket.dirty {
		return
	}
	ticket.dirty = true
	l.tokens <- ticket.token
}
