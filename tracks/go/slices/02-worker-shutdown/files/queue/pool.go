package queue

import "context"

// Handler processes one job.
type Handler func(ctx context.Context, j Job) error

// RunPool starts n workers that pull from jobs and call handle.
// It returns how many jobs were handled once jobs is closed and drained,
// or once ctx is cancelled — and never before every worker has exited.
func RunPool(ctx context.Context, n int, jobs <-chan Job, handle Handler) int {
	// TODO: start n goroutines; each selects on <-ctx.Done() and <-jobs.
	// Count handled jobs safely (sync/atomic), wait for all workers (sync.WaitGroup).
	return 0
}
