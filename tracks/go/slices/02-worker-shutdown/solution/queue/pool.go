package queue

import (
	"context"
	"sync"
	"sync/atomic"
)

// Handler processes one job.
type Handler func(ctx context.Context, j Job) error

// RunPool starts n workers that pull from jobs and call handle.
// It returns how many jobs were handled once jobs is closed and drained,
// or once ctx is cancelled — and never before every worker has exited.
func RunPool(ctx context.Context, n int, jobs <-chan Job, handle Handler) int {
	var handled atomic.Int64
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				select {
				case <-ctx.Done():
					return
				case j, ok := <-jobs:
					if !ok {
						return
					}
					_ = handle(ctx, j)
					handled.Add(1)
				}
			}
		}()
	}
	wg.Wait()
	return int(handled.Load())
}
