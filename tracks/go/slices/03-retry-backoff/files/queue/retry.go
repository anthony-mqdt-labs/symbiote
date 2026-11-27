package queue

import (
	"context"
	"time"
)

// Backoff returns the delay before retry attempt n (0-based): base·2ⁿ, capped at max.
// It must not overflow for large n.
func Backoff(n int, base, max time.Duration) time.Duration {
	// TODO
	return 0
}

// Retry calls fn up to attempts times, waiting Backoff(i, base, max) after each failure.
// If ctx is cancelled while waiting, it returns ctx.Err() wrapped together with the last error.
// When attempts run out it returns "retry: gave up after N attempts: <last error>" (wrapped).
func Retry(ctx context.Context, attempts int, base, max time.Duration, fn func() error) error {
	// TODO
	return nil
}
