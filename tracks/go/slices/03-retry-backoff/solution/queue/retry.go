package queue

import (
	"context"
	"fmt"
	"time"
)

// Backoff returns the delay before retry attempt n (0-based): base·2ⁿ, capped at max.
// It must not overflow for large n.
func Backoff(n int, base, max time.Duration) time.Duration {
	d := base
	for i := 0; i < n && d < max; i++ {
		d *= 2
	}
	if d > max {
		return max
	}
	return d
}

// Retry calls fn up to attempts times, waiting Backoff(i, base, max) after each failure.
// If ctx is cancelled while waiting, it returns ctx.Err() wrapped together with the last error.
// When attempts run out it returns "retry: gave up after N attempts: <last error>" (wrapped).
func Retry(ctx context.Context, attempts int, base, max time.Duration, fn func() error) error {
	var last error
	for i := 0; i < attempts; i++ {
		if last = fn(); last == nil {
			return nil
		}
		if i == attempts-1 {
			break
		}
		select {
		case <-time.After(Backoff(i, base, max)):
		case <-ctx.Done():
			return fmt.Errorf("%w: %w", ctx.Err(), last)
		}
	}
	return fmt.Errorf("retry: gave up after %d attempts: %w", attempts, last)
}
