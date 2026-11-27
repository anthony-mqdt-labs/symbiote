package queue

import (
	"errors"
	"testing"
)

func TestValidate(t *testing.T) {
	cases := []struct {
		name string
		id   string
		want error
	}{
		{"ok", "job-1", nil},
		{"empty", "", ErrMissingID},
		{"whitespace", " \t ", ErrMissingID},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := (Job{ID: c.id}).Validate(); !errors.Is(got, c.want) {
				t.Fatalf("Validate(%q) = %v, want %v", c.id, got, c.want)
			}
		})
	}
}
