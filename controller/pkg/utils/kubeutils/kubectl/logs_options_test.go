package kubectl

import (
	"slices"
	"testing"
	"time"
)

func TestBuildLogArgs_NoOptions(t *testing.T) {
	args := BuildLogArgs()
	if len(args) != 0 {
		t.Errorf("expected empty args, got %v", args)
	}
}

func TestBuildLogArgs_IndividualOptions(t *testing.T) {
	tests := []struct {
		name         string
		option       LogOption
		expectedArgs []string
	}{
		{
			name:         "WithContainer",
			option:       WithContainer("my-container"),
			expectedArgs: []string{"-c", "my-container"},
		},
		{
			name:         "WithSince",
			option:       WithSince(5 * time.Minute),
			expectedArgs: []string{"--since", "5m0s"},
		},
		{
			name:         "WithSinceTime",
			option:       WithSinceTime("2024-01-01T00:00:00Z"),
			expectedArgs: []string{"--since-time", "2024-01-01T00:00:00Z"},
		},
		{
			name:         "WithTail",
			option:       WithTail(100),
			expectedArgs: []string{"--tail", "100"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			args := BuildLogArgs(tt.option)
			for _, expected := range tt.expectedArgs {
				if !slices.Contains(args, expected) {
					t.Errorf("expected args to contain %q, got %v", expected, args)
				}
			}
		})
	}
}

func TestBuildLogArgs_MultipleOptions(t *testing.T) {
	args := BuildLogArgs(
		WithContainer("app"),
		WithTail(50),
		WithSince(10*time.Minute),
	)

	expectedPairs := [][]string{
		{"-c", "app"},
		{"--tail", "50"},
		{"--since", "10m0s"},
	}

	for _, pair := range expectedPairs {
		for _, expected := range pair {
			if !slices.Contains(args, expected) {
				t.Errorf("expected args to contain %q, got %v", expected, args)
			}
		}
	}
}

func TestBuildLogArgs_DoesNotIncludeTailWhenNegative(t *testing.T) {
	args := BuildLogArgs(WithContainer("app"))
	if slices.Contains(args, "--tail") {
		t.Errorf("expected args not to contain --tail, got %v", args)
	}
}

func TestBuildLogArgs_DoesNotIncludeSinceWhenZero(t *testing.T) {
	args := BuildLogArgs(WithContainer("app"))
	if slices.Contains(args, "--since") {
		t.Errorf("expected args not to contain --since, got %v", args)
	}
}

func TestBuildLogArgs_DoesNotIncludeSinceTimeWhenEmpty(t *testing.T) {
	args := BuildLogArgs(WithContainer("app"))
	if slices.Contains(args, "--since-time") {
		t.Errorf("expected args not to contain --since-time, got %v", args)
	}
}

func TestFormatSinceTime(t *testing.T) {
	testTime := time.Date(2024, 1, 1, 12, 30, 45, 0, time.UTC)
	formatted := FormatSinceTime(testTime)
	expected := "2024-01-01T12:30:45Z"

	if formatted != expected {
		t.Errorf("expected %q, got %q", expected, formatted)
	}
}
