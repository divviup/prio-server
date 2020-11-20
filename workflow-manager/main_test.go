package main

import "testing"

func TestIdForJobName(t *testing.T) {
	input := "FooBar%012345678901234567890123456789"
	id := idForJobName(input)
	expected := "foobar-01234567890123456789012"
	if id != expected {
		t.Errorf("expected id %q, got %q", expected, id)
	}
}
