//go:build e2e

package e2e_test

import (
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestRBACHeaderAuthorization(tt *testing.T) {
	t := New(tt)

	t.Apply(manifest("rbac", "cel-rbac.yaml"))
	t.HTTPRouteAccepted("httpbin-route", base.Namespace)

	t.Send(
		"httpbin/get",
		base.ExpectForbidden(gomega.ContainSubstring("authorization failed")),
	)
	t.Send(
		"httpbin/get",
		base.ExpectOK(),
		curl.WithHeader("x-my-header", "cool-beans"),
	)
}
