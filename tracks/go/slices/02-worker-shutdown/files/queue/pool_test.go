package queue

import (
	"context"
	"fmt"
	"sync/atomic"
	"testing"
	"time"
)

func TestPoolDrains(t *testing.T) {
	jobs := make(chan Job, 10)
	for i := 0; i < 10; i++ {
		jobs <- Job{ID: fmt.Sprint(i)}
	}
	close(jobs)
	var seen atomic.Int32
	got := RunPool(context.Background(), 3, jobs, func(ctx context.Context, j Job) error {
		seen.Add(1)
		return nil
	})
	if got != 10 || seen.Load() != 10 {
		t.Fatalf("RunPool handled %d (handler saw %d), want 10", got, seen.Load())
	}
}

func TestPoolShutdown(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	jobs := make(chan Job) // never closed: only ctx can stop the pool
	var inflight atomic.Int32
	done := make(chan int, 1)
	go func() {
		done <- RunPool(ctx, 4, jobs, func(ctx context.Context, j Job) error {
			inflight.Add(1)
			defer inflight.Add(-1)
			time.Sleep(20 * time.Millisecond)
			return nil
		})
	}()
	select {
	case jobs <- Job{ID: "a"}:
	case <-time.After(time.Second):
		t.Fatal("no worker received a job within 1s")
	}
	cancel()
	select {
	case n := <-done:
		if n != 1 {
			t.Fatalf("handled %d jobs, want 1 (the in-flight job must finish)", n)
		}
		if inflight.Load() != 0 {
			t.Fatal("RunPool returned while a handler was still running")
		}
	case <-time.After(time.Second):
		t.Fatal("RunPool did not return within 1s of cancel: goroutine leak")
	}
}
