//go:build e2e

package e2e_test

import (
	"fmt"
	"net/http"
	"strings"
	"testing"

	"github.com/onsi/gomega"
	"istio.io/istio/pkg/test/util/retry"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/gomega/transforms"
)

func TestBackendAuth(tt *testing.T) {
	t := New(tt)

	t.Run("Credentials", func(t base.Test) {
		testBackendAuthCredentials(t)
	})
	t.Run("JwtSign", func(t base.Test) {
		t.Run("Valid", func(t base.Test) {
			testValidJwtSign(t)
		})
		t.Run("Invalid", func(t base.Test) {
			testInvalidJwtSign(t)
		})
	})
}

func testBackendAuthCredentials(t base.Test) {
	t.Apply(manifest("backendauth", "credentials.yaml"))

	t.Send("credentials-auth.example.com", &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		Body: gomega.WithTransform(transforms.WithEchoHeaders(),
			gomega.And(
				gomega.HaveKeyWithValue("Dd-Api-Key", "primary-api-key"),
				gomega.HaveKeyWithValue("Dd-Application-Key", "application-key"),
			),
		),
	})
}

func testValidJwtSign(t base.Test) {
	t.Apply(manifest("backendauth", "valid-jwt-sign.yaml"))
	t.HTTPRouteAccepted("route-backendauth-valid-jwt-sign", base.Namespace)

	t.Send("valid-jwt-sign.example.com", &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		Body: gomega.WithTransform(
			transforms.WithEchoHeaders(),
			gomega.HaveKeyWithValue(
				"Authorization",
				gomega.MatchRegexp(`^Bearer [^.]+\.[^.]+\.[^.]+$`),
			),
		),
	})
}

func testInvalidJwtSign(t base.Test) {
	const missingKeyRef = "jwt-sign-secret-missing"
	t.Apply(manifest("backendauth", "invalid-jwt-sign.yaml"))
	t.HTTPRouteAccepted("route-backendauth-invalid-jwt-sign", base.Namespace)

	retry.UntilSuccessOrFail(t, func() error {
		policy := &agentgateway.AgentgatewayPolicy{}
		if err := t.TestInstallation.ClusterContext.ControllerClient.Get(
			t.Ctx,
			types.NamespacedName{Name: "backendauth-invalid-jwt-sign", Namespace: base.Namespace},
			policy,
		); err != nil {
			return err
		}
		for _, ancestor := range policy.Status.Ancestors {
			for _, condition := range ancestor.Conditions {
				if condition.Type == string(agentgateway.PolicyConditionAccepted) &&
					condition.Status == metav1.ConditionTrue &&
					condition.Reason == string(agentgateway.PolicyReasonPartiallyValid) &&
					strings.Contains(condition.Message, missingKeyRef) {
					return nil
				}
			}
		}
		return fmt.Errorf("policy status does not report the missing jwtSign secret: %+v", policy.Status)
	})

	t.Send("invalid-jwt-sign.example.com", &testmatchers.HttpResponse{
		StatusCode: http.StatusInternalServerError,
		Body: gomega.And(
			gomega.ContainSubstring("backend authentication failed: jwtSign configuration is invalid"),
			gomega.Not(gomega.ContainSubstring(missingKeyRef)),
		),
	})
}
