package plugins

import (
	"testing"
)

func TestNonEmptyStringPtr(t *testing.T) {
	t.Run("nil returns nil", func(t *testing.T) {
		if nonEmptyStringPtr(nil) != nil {
			t.Error("expected nil for nil input")
		}
	})
	t.Run("empty string returns nil", func(t *testing.T) {
		s := ""
		if nonEmptyStringPtr(&s) != nil {
			t.Error("expected nil for empty string")
		}
	})
	t.Run("non-empty string returns pointer", func(t *testing.T) {
		s := "abc123"
		result := nonEmptyStringPtr(&s)
		if result == nil || *result != "abc123" {
			t.Errorf("expected %q, got %v", s, result)
		}
	})
}
