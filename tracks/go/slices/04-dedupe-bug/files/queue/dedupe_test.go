package queue

import (
	"testing"
	"time"
)

func TestDeduper(t *testing.T) {
	clock := time.Unix(0, 0)
	d := NewDeduper(time.Minute)
	d.now = func() time.Time { return clock }

	steps := []struct {
		id      string
		advance time.Duration
		want    bool
	}{
		{"Job-1", 0, true},
		{"Job-1", 0, false},
		{"job-1", 0, false},
		{"JOB-1", 30 * time.Second, false},
		{"job-2", 0, true},
		{"Job-1", 2 * time.Minute, true},
	}
	for i, s := range steps {
		clock = clock.Add(s.advance)
		if got := d.Admit(Job{ID: s.id}); got != s.want {
			t.Fatalf("step %d: Admit(%q) = %v, want %v", i, s.id, got, s.want)
		}
	}
}
