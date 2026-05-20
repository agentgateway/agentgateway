//go:build e2e

package base

import (
	"os"
	"testing"
	"time"
)

const traceEnv = "AGW_E2E_TRACE"

func traceEnabled() bool {
	switch os.Getenv(traceEnv) {
	case "1", "true", "TRUE", "yes", "YES":
		return true
	default:
		return false
	}
}

func tracef(t *testing.T, format string, args ...any) {
	t.Helper()
	if traceEnabled() {
		t.Logf(format, args...)
	}
}

func TraceStep(t *testing.T, format string, args ...any) func() {
	t.Helper()
	return traceStep(t, format, args...)
}

func traceStep(t *testing.T, format string, args ...any) func() {
	t.Helper()
	if !traceEnabled() {
		return func() {}
	}
	start := time.Now()
	return func() {
		t.Helper()
		t.Logf(format+" in %s", append(args, time.Since(start).Round(time.Millisecond))...)
	}
}
