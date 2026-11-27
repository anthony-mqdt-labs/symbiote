package queue

import (
	"errors"
	"io"
	"strings"
	"testing"
)

type countingReader struct {
	r io.Reader
	n int
}

func (c *countingReader) Read(p []byte) (int, error) {
	n, err := c.r.Read(p)
	c.n += n
	return n, err
}

func TestDecodeValid(t *testing.T) {
	j, err := DecodeJob(strings.NewReader(`{"id":"a","payload":"p"}`))
	if err != nil || j.ID != "a" || j.Payload != "p" {
		t.Fatalf("DecodeJob = %+v, %v", j, err)
	}
}

func TestDecodeRejects(t *testing.T) {
	for name, in := range map[string]string{
		"unknown field": `{"id":"a","admin":true}`,
		"trailing data": `{"id":"a"}{"id":"b"}`,
		"not json":      `id=a`,
		"wrong type":    `{"id":7}`,
	} {
		if _, err := DecodeJob(strings.NewReader(in)); err == nil {
			t.Errorf("%s: DecodeJob(%s) accepted invalid input", name, in)
		}
	}
	if _, err := DecodeJob(strings.NewReader(`{"id":"  "}`)); !errors.Is(err, ErrMissingID) {
		t.Errorf("blank id: err = %v, want ErrMissingID", err)
	}
}

func TestDecodeTooLarge(t *testing.T) {
	huge := `{"id":"a","payload":"` + strings.Repeat("x", 4*MaxJobBytes) + `"}`
	cr := &countingReader{r: strings.NewReader(huge)}
	_, err := DecodeJob(cr)
	if !errors.Is(err, ErrTooLarge) {
		t.Fatalf("err = %v, want ErrTooLarge", err)
	}
	if cr.n > MaxJobBytes+1 {
		t.Fatalf("read %d bytes, want at most MaxJobBytes+1 (%d)", cr.n, MaxJobBytes+1)
	}
}
