package queue

import (
	"errors"
	"strings"
)

// ErrMissingID is returned when a job has no usable ID.
var ErrMissingID = errors.New("job: missing id")

// Validate reports whether j can be enqueued.
// A job needs an ID; whitespace-only IDs count as missing.
func (j Job) Validate() error {
	if strings.TrimSpace(j.ID) == "" {
		return ErrMissingID
	}
	return nil
}
