//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/gomega/transforms"
)

func TestOAuthTokenExchange(tt *testing.T) {
	t := New(tt)
	t.Apply(manifest("oauth", "routes.yaml"))

	t.HTTPRouteAccepted("cross-app-access", base.Namespace)
	t.HTTPRouteAccepted("oauth-token-exchange", base.Namespace)
	t.HTTPRouteAccepted("oauth-jwt-bearer", base.Namespace)

	assertions.EventuallyAgwPolicyCondition(t, "cross-app-access", base.Namespace, "Accepted", metav1.ConditionTrue)
	t.Run("CrossAppAccess", func(t base.Test) {
		t.Send("cross-app-access.com",
			&testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.HaveKeyWithValue("Authorization", "Bearer cross-app-access-token"),
				),
			},
			curl.WithHeader("Authorization", "Bearer subject-id-token"),
		)
	})

	assertions.EventuallyAgwPolicyCondition(t, "oauth-token-exchange", base.Namespace, "Accepted", metav1.ConditionTrue)
	t.Run("TokenExchange", func(t base.Test) {
		t.Send("oauth-token-exchange.com",
			&testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.HaveKeyWithValue("Authorization", "Bearer token-exchange-access"),
				),
			},
			curl.WithHeader("Authorization", "Bearer subject-token"),
			curl.WithHeader("X-Actor-Token", "actor-token"),
			curl.WithHeader("X-Tenant", "tenant-a"),
		)
	})

	t.Run("MissingSubjectToken", func(t base.Test) {
		t.Send("oauth-token-exchange.com",
			base.Expect(http.StatusBadRequest),
			curl.WithHeader("X-Actor-Token", "actor-token"),
			curl.WithHeader("X-Tenant", "tenant-a"),
		)
	})

	assertions.EventuallyAgwPolicyCondition(t, "oauth-jwt-bearer", base.Namespace, "Accepted", metav1.ConditionTrue)
	t.Run("JWTBearer", func(t base.Test) {
		t.Send("oauth-jwt-bearer.com",
			&testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.HaveKeyWithValue("X-Exchanged-Token", "jwt-bearer-access"),
				),
			},
			curl.WithHeader("X-Client-Assertion", "jwt-assertion"),
		)
	})
}
