package queue

import "errors"

// ErrFull is returned when the queue is at capacity.
// Callers decide what backpressure means for them: retry, shed, or block elsewhere.
var ErrFull = errors.New("queue: full")

// Queue is a bounded in-memory FIFO of jobs.
type Queue struct {
	jobs chan Job
}

// New returns a queue holding at most capacity jobs.
func New(capacity int) *Queue {
	return &Queue{jobs: make(chan Job, capacity)}
}

// TryEnqueue adds j without ever blocking. It returns ErrFull at capacity.
func (q *Queue) TryEnqueue(j Job) error {
	select {
	case q.jobs <- j:
		return nil
	default:
		return ErrFull
	}
}

// Len reports how many jobs are waiting.
func (q *Queue) Len() int {
	return len(q.jobs)
}

// Jobs is the receive side workers consume.
func (q *Queue) Jobs() <-chan Job { return q.jobs }
