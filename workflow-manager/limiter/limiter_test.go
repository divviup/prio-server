package limiter

import (
	"testing"
	"time"
)

func TestTicketDoubleDone(t *testing.T) {
	l := New(5)
	doneChan := make(chan struct{})
	go l.Execute(func(ticket *Ticket) {
		l.Done(ticket)
		l.Done(ticket)
		doneChan <- struct{}{}
	})
	for {
		select {
		case <-doneChan:
			// success
			return
		case <-time.After(time.Second):
			t.Fatal("timeout waiting for job to complete")
		}
	}
}

func TestLimiterBehavior(t *testing.T) {
	firstJobChannel := make(chan struct{})

	l := New(1)
	l.Execute(func(ticket *Ticket) {
		go func() {
			// Indicate this job is running and thus occupying the Limiter's
			// single token, then block until instructed to terminate
			firstJobChannel <- struct{}{}
			t.Log("first job running")

			// This will synchronize later in the code with signaledFirstJob
			<-firstJobChannel
			l.Done(ticket)
			t.Log("first job done")
		}()
	})

	// Don't allow scheduling of more jobs until we know first job is running
	t.Log("waiting for first job")
	<-firstJobChannel
	// From this point, we know for certain that the first job is running, and
	// occupying the Limiter's single token
	t.Log("first job is running")

	// Add two more jobs -- neither should be able to run until the first one
	// is finished. Call l.Execute() in a goroutine because otherwise those
	// calls will block
	messageChannel := make(chan string)
	go func() {
		l.Execute(func(ticket *Ticket) {
			go func() {
				defer l.Done(ticket)
				// Indicate that this job has started. Send a string to make the
				// switch below more readable
				messageChannel <- "second job"
				l.Done(ticket)
				t.Log("second job done")
			}()
		})
	}()

	go func() {
		l.Execute(func(ticket *Ticket) {
			go func() {
				defer l.Done(ticket)
				// Indicate that this job has started
				messageChannel <- "third job"
				t.Log("third job done")
			}()
		})
	}()

	signaledFirstJob := false
	secondJobRan := false
	thirdJobRan := false

	// Wait 10 milliseconds before signaling the first job to complete. In the
	// intervening period, we should not see either the second or third jobs
	// run.
	for {
		select {
		case message := <-messageChannel:
			switch message {
			case "second job":
				if !signaledFirstJob {
					t.Error("second job ran before first job signaled")
				}
				// Limiter makes no guarantees about the order in which jobs
				// will be run, so we handle either order of execution of the
				// second and third jobs
				secondJobRan = true
				if thirdJobRan {
					t.Log("second and third jobs ran -- success")
					return
				}
			case "third job":
				if !signaledFirstJob {
					t.Error("third job ran before first job signaled")
				}
				thirdJobRan = true
				if secondJobRan {
					t.Log("second and third jobs ran -- success")
					return
				}
			}
		case <-time.After(10 * time.Millisecond):
			if signaledFirstJob {
				// This case will fire  every 10 milliseconds, so ignore it
				// after the first time
				continue
			}
			if secondJobRan || thirdJobRan {
				t.Error("second or third job ran before first job completed")
			}
			// Signal to the first job it should complete. We know for certain
			// that the Limiter's single token is occupied until this point.
			signaledFirstJob = true
			firstJobChannel <- struct{}{}
		case <-time.After(time.Second):
			t.Error("timeout waiting for test to finish")
		}
	}
}
