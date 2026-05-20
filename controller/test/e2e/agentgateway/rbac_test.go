//go:build e2e

package agentgateway

import (
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

func TestRBACHeaderAuthorization(t *testing.T) {
	agw := New(t)

	agw.Apply(manifest("rbac", "cel-rbac.yaml"))
	agw.HTTPRouteAccepted("httpbin-route", base.Namespace)

	agw.Send(
		"httpbin/get",
		base.ExpectForbidden(gomega.ContainSubstring("authorization failed")),
	)
	agw.Send(
		"httpbin/get",
		base.ExpectOK(),
		curl.WithHeader("x-my-header", "cool-beans"),
	)
}
