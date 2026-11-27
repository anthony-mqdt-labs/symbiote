package queue

import (
	"encoding/json"
	"errors"
	"fmt"
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
	lr := &io.LimitedReader{R: r, N: MaxJobBytes + 1}
	dec := json.NewDecoder(lr)
	dec.DisallowUnknownFields()
	var j Job
	if err := dec.Decode(&j); err != nil {
		if lr.N <= 0 {
			return Job{}, ErrTooLarge
		}
		return Job{}, fmt.Errorf("decode: %w", err)
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		if lr.N <= 0 {
			return Job{}, ErrTooLarge
		}
		return Job{}, errors.New("decode: trailing data after job")
	}
	if err := j.Validate(); err != nil {
		return Job{}, err
	}
	return j, nil
}
