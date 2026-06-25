//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/gomega/transforms"
)

func TestBackendAuth(tt *testing.T) {
	t := New(tt)

	t.Run("MultiHeader", func(t base.Test) {
		testBackendAuthMultiHeader(t)
	})
}

func testBackendAuthMultiHeader(t base.Test) {
	t.Apply(manifest("backendauth", "multi-header.yaml"))

	t.Send("multi-header-auth.example.com", &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		Body: gomega.WithTransform(transforms.WithEchoHeaders(),
			gomega.And(
				gomega.HaveKeyWithValue("Dd-Api-Key", "primary-api-key"),
				gomega.HaveKeyWithValue("Dd-Application-Key", "application-key"),
			),
		),
	})
}
