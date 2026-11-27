package queue

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

var errBoom = errors.New("boom")

func TestBackoff(t *testing.T) {
	base, max := 10*time.Millisecond, time.Second
	for n, want := range map[int]time.Duration{0: 10 * time.Millisecond, 1: 20 * time.Millisecond, 3: 80 * time.Millisecond, 7: time.Second, 70: time.Second} {
		if got := Backoff(n, base, max); got != want {
			t.Errorf("Backoff(%d) = %v, want %v", n, got, want)
		}
	}
}

func TestRetryEventuallySucceeds(t *testing.T) {
	calls := 0
	err := Retry(context.Background(), 5, time.Millisecond, 5*time.Millisecond, func() error {
		calls++
		if calls < 3 {
			return errBoom
		}
		return nil
	})
	if err != nil || calls != 3 {
		t.Fatalf("err=%v calls=%d, want nil and 3", err, calls)
	}
}

func TestRetryGivesUp(t *testing.T) {
	calls := 0
	err := Retry(context.Background(), 3, time.Millisecond, time.Millisecond, func() error { calls++; return errBoom })
	if calls != 3 || !errors.Is(err, errBoom) || err == nil || !strings.Contains(err.Error(), "gave up after 3 attempts") {
		t.Fatalf("calls=%d err=%v, want 3 calls and a wrapped errBoom mentioning the attempt count", calls, err)
	}
}

func TestRetryCancel(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	time.AfterFunc(20*time.Millisecond, cancel)
	start := time.Now()
	err := Retry(ctx, 5, time.Second, time.Second, func() error { return errBoom })
	if time.Since(start) > 500*time.Millisecond {
		t.Fatalf("Retry ignored cancellation for %v", time.Since(start))
	}
	if !errors.Is(err, context.Canceled) || !errors.Is(err, errBoom) {
		t.Fatalf("err = %v, want both context.Canceled and errBoom in the chain", err)
	}
}
