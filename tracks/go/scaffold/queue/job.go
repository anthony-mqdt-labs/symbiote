// Package queue is a durable job queue, built one verified slice at a time.
package queue

// Job is one unit of work.
type Job struct {
	ID       string `json:"id"`
	Payload  string `json:"payload"`
	Attempts int    `json:"attempts"`
}
