package queue

import "errors"

// ErrMissingID is returned when a job has no usable ID.
var ErrMissingID = errors.New("job: missing id")

// Validate reports whether j can be enqueued.
// A job needs an ID; whitespace-only IDs count as missing.
func (j Job) Validate() error {
	// TODO: return ErrMissingID when j.ID is empty or only whitespace.
	return nil
}
