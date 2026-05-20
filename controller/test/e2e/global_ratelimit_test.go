//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

// Run a tiny burst so all checks stay in one fixed RL window.
// The external rate limiter uses clock-aligned windows, so long loops can
// straddle the boundary and flake.
const rlBurstTries = 3

func TestGlobalRateLimit(t *testing.T) {
	agw := New(t)
	agw.Apply(
		globalRateLimitManifest("rate-limit-server.yaml"),
		globalRateLimitManifest("routes.yaml"),
	)

	agw.Run("ByRemoteAddress", func() {
		testGlobalRateLimitByRemoteAddress(agw)
	})
	agw.Run("ByPath", func() {
		testGlobalRateLimitByPath(agw)
	})
	agw.Run("ByUserID", func() {
		testGlobalRateLimitByUserID(agw)
	})
	agw.Run("CombinedLocalAndGlobal", func() {
		testCombinedLocalAndGlobalRateLimit(agw)
	})
}

func testGlobalRateLimitByRemoteAddress(agw *base.BaseTestingSuite) {
	agw.Apply(
		globalRateLimitManifest("ip-rate-limit.yaml"),
	)

	agw.Send("example.com/path1", base.ExpectOK())
	assertConsistentRateLimitResponse(agw, "example.com/path1", http.StatusTooManyRequests)
	assertConsistentRateLimitResponse(agw, "example.com/path2", http.StatusTooManyRequests)
}

func testGlobalRateLimitByPath(agw *base.BaseTestingSuite) {
	agw.Apply(
		globalRateLimitManifest("path-rate-limit.yaml"),
	)

	agw.Send("example.com/path1", base.ExpectOK())
	assertConsistentRateLimitResponse(agw, "example.com/path1", http.StatusTooManyRequests)
	assertConsistentRateLimitResponse(agw, "example.com/path2", http.StatusOK)
}

func testGlobalRateLimitByUserID(agw *base.BaseTestingSuite) {
	agw.Apply(
		globalRateLimitManifest("user-rate-limit.yaml"),
	)

	agw.Send("example.com/path1", base.ExpectOK(), curl.WithHeader("X-User-ID", "user1"))
	assertConsistentRateLimitResponse(agw, "example.com/path1", http.StatusTooManyRequests, curl.WithHeader("X-User-ID", "user1"))
	agw.Send("example.com/path1", base.ExpectOK(), curl.WithHeader("X-User-ID", "user2"))
}

func testCombinedLocalAndGlobalRateLimit(agw *base.BaseTestingSuite) {
	agw.Apply(
		globalRateLimitManifest("combined-rate-limit.yaml"),
	)

	agw.Send("example.com/path1", base.ExpectOK())
	assertConsistentRateLimitResponse(agw, "example.com/path1", http.StatusTooManyRequests)
}

func globalRateLimitManifest(name string) string {
	return manifest("rate-limit", "global", name)
}

func assertConsistentRateLimitResponse(t *base.BaseTestingSuite, target string, status int, opts ...curl.Option) {
	for range rlBurstTries {
		t.Send(target, base.Expect(status), opts...)
	}
}
