//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const (
	hostRewriteBackendHost = "backend.agentgateway-base.svc.cluster.local"
	svcDefaultHost         = "svc-default.hostrewrite.test"
	svcNoneHost            = "svc-none.hostrewrite.test"
	svcAutoHost            = "svc-auto.hostrewrite.test"
	agbeDefaultHost        = "agbe-default.hostrewrite.test"
	agbeNoneHost           = "agbe-none.hostrewrite.test"
	agbeAutoHost           = "agbe-auto.hostrewrite.test"
)

func TestHostRewrite(tt *testing.T) {
	t := New(tt)

	t.Run("ServiceBackend", func(t base.Test) {
		t.Apply(manifest("hostrewrite", "service-routes.yaml"))

		assertHostRewrite(t, svcDefaultHost, svcDefaultHost)
		assertHostRewrite(t, svcNoneHost, svcNoneHost)
		assertHostRewrite(t, svcAutoHost, hostRewriteBackendHost)
	})
	t.Run("AgentgatewayBackend", func(t base.Test) {
		t.Apply(manifest("hostrewrite", "agbe-routes.yaml"))

		assertHostRewrite(t, agbeDefaultHost, hostRewriteBackendHost)
		assertHostRewrite(t, agbeNoneHost, agbeNoneHost)
		assertHostRewrite(t, agbeAutoHost, hostRewriteBackendHost)
	})
}

func assertHostRewrite(t base.Test, routeHost string, expectedUpstreamHost string) {
	t.Helper()
	t.Send(routeHost,
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring("Host=" + expectedUpstreamHost),
		},
	)
}
