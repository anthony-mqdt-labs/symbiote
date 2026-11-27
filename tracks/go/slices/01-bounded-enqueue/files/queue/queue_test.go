package queue

import (
	"errors"
	"sync"
	"sync/atomic"
	"testing"
)

func TestEnqueueBounded(t *testing.T) {
	q := New(2)
	for _, id := range []string{"a", "b"} {
		if err := q.TryEnqueue(Job{ID: id}); err != nil {
			t.Fatalf("TryEnqueue(%s) = %v, want nil", id, err)
		}
	}
	if err := q.TryEnqueue(Job{ID: "c"}); !errors.Is(err, ErrFull) {
		t.Fatalf("third TryEnqueue = %v, want ErrFull", err)
	}
	if n := q.Len(); n != 2 {
		t.Fatalf("Len() = %d, want 2", n)
	}
	if j := <-q.Jobs(); j.ID != "a" {
		t.Fatalf("first job out = %q, want a (FIFO)", j.ID)
	}
}

func TestEnqueueConcurrent(t *testing.T) {
	q := New(5)
	var ok, full atomic.Int32
	var wg sync.WaitGroup
	for i := 0; i < 20; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := q.TryEnqueue(Job{ID: "x"}); err == nil {
				ok.Add(1)
			} else if errors.Is(err, ErrFull) {
				full.Add(1)
			}
		}()
	}
	wg.Wait()
	if ok.Load() != 5 || full.Load() != 15 {
		t.Fatalf("accepted=%d rejected=%d, want 5 and 15", ok.Load(), full.Load())
	}
}
