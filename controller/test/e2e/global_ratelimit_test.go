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

func TestGlobalRateLimit(tt *testing.T) {
	t := New(tt)
	t.ApplyPersistent(globalRateLimitManifest("rate-limit-server.yaml"))
	t.Apply(globalRateLimitManifest("routes.yaml"))

	t.Run("ByRemoteAddress", func(t base.Test) {
		testGlobalRateLimitByRemoteAddress(t)
	})
	t.Run("ByPath", func(t base.Test) {
		testGlobalRateLimitByPath(t)
	})
	t.Run("ByUserID", func(t base.Test) {
		testGlobalRateLimitByUserID(t)
	})
	t.Run("CombinedLocalAndGlobal", func(t base.Test) {
		testCombinedLocalAndGlobalRateLimit(t)
	})
}

func testGlobalRateLimitByRemoteAddress(t base.Test) {
	t.Apply(
		globalRateLimitManifest("ip-rate-limit.yaml"),
	)

	t.Send("example.com/path1", base.ExpectOK())
	assertConsistentRateLimitResponse(t, "example.com/path1", http.StatusTooManyRequests)
	assertConsistentRateLimitResponse(t, "example.com/path2", http.StatusTooManyRequests)
}

func testGlobalRateLimitByPath(t base.Test) {
	t.Apply(
		globalRateLimitManifest("path-rate-limit.yaml"),
	)

	t.Send("example.com/path1", base.ExpectOK())
	assertConsistentRateLimitResponse(t, "example.com/path1", http.StatusTooManyRequests)
	assertConsistentRateLimitResponse(t, "example.com/path2", http.StatusOK)
}

func testGlobalRateLimitByUserID(t base.Test) {
	t.Apply(
		globalRateLimitManifest("user-rate-limit.yaml"),
	)

	t.Send("example.com/path1", base.ExpectOK(), curl.WithHeader("X-User-ID", "user1"))
	assertConsistentRateLimitResponse(t, "example.com/path1", http.StatusTooManyRequests, curl.WithHeader("X-User-ID", "user1"))
	t.Send("example.com/path1", base.ExpectOK(), curl.WithHeader("X-User-ID", "user2"))
}

func testCombinedLocalAndGlobalRateLimit(t base.Test) {
	t.Apply(
		globalRateLimitManifest("combined-rate-limit.yaml"),
	)

	t.Send("example.com/path1", base.ExpectOK())
	assertConsistentRateLimitResponse(t, "example.com/path1", http.StatusTooManyRequests)
}

func globalRateLimitManifest(name string) string {
	return manifest("rate-limit", "global", name)
}

func assertConsistentRateLimitResponse(t base.Test, target string, status int, opts ...curl.Option) {
	for range rlBurstTries {
		t.Send(target, base.Expect(status), opts...)
	}
}
