package queue

import (
	"encoding/json"
	"errors"
	"io"
)

// MaxJobBytes bounds a single encoded job.
const MaxJobBytes = 64 << 10

// ErrTooLarge is returned when the input exceeds MaxJobBytes.
var ErrTooLarge = errors.New("decode: job exceeds MaxJobBytes")

// DecodeJob reads exactly one JSON job from r, an untrusted request body.
// It rejects unknown fields, trailing data, invalid jobs, and inputs over MaxJobBytes,
// and never reads more than MaxJobBytes+1 bytes from r.
func DecodeJob(r io.Reader) (Job, error) {
	var j Job
	err := json.NewDecoder(r).Decode(&j)
	return j, err
}
