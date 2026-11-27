package queue

import (
	"strings"
	"sync"
	"time"
)

// Deduper drops jobs whose ID was admitted within the window.
// IDs are case-insensitive: "Job-1" and "job-1" are the same job.
type Deduper struct {
	mu     sync.Mutex
	seen   map[string]time.Time
	window time.Duration
	now    func() time.Time
}

// NewDeduper returns a Deduper that remembers IDs for window.
func NewDeduper(window time.Duration) *Deduper {
	return &Deduper{seen: map[string]time.Time{}, window: window, now: time.Now}
}

// Admit reports whether j should run, recording it if so.
func (d *Deduper) Admit(j Job) bool {
	d.mu.Lock()
	defer d.mu.Unlock()
	key := strings.ToLower(j.ID)
	if t, ok := d.seen[j.ID]; ok && d.now().Sub(t) < d.window {
		return false
	}
	d.seen[key] = d.now()
	return true
}
