//go:build e2e

package base

import (
	"slices"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
)

func TestTargetOptions(t *testing.T) {
	args := curl.BuildArgs(targetOptions(t, "httpbin/get")...)
	requireContains(t, args, "Host: httpbin")
	requireContains(t, args, "http://127.0.0.1:8080/get")

	args = curl.BuildArgs(targetOptions(t, "https://httpbin/get?debug=true")...)
	requireContains(t, args, "Host: httpbin")
	requireContains(t, args, "https://127.0.0.1:8080/get?debug=true")
}

func requireContains(t *testing.T, values []string, want string) {
	t.Helper()
	if !slices.Contains(values, want) {
		t.Fatalf("expected %q in %v", want, values)
	}
}
