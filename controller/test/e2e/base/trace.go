//go:build e2e

package base

import (
	"time"

	"istio.io/istio/pkg/test"

	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

func traceEnabled() bool {
	return testutils.ShouldTraceE2E()
}

func TraceStep(t test.Failer, format string, args ...any) func() {
	t.Helper()
	return traceStep(t, format, args...)
}

func traceStep(t test.Failer, format string, args ...any) func() {
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
