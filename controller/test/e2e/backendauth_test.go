//go:build e2e

package e2e_test

import (
	"fmt"
	"net/http"
	"strings"
	"testing"

	"github.com/onsi/gomega"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
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

	assertions.EventuallyAgwPolicyStatus(t, "backendauth-invalid-jwt-sign", base.Namespace, func(status gwv1.PolicyStatus) error {
		for _, ancestor := range status.Ancestors {
			for _, condition := range ancestor.Conditions {
				if condition.Type == agentgateway.PolicyConditionAccepted &&
					condition.Status == metav1.ConditionTrue &&
					condition.Reason == agentgateway.PolicyReasonPartiallyValid &&
					strings.Contains(condition.Message, missingKeyRef) {
					return nil
				}
			}
		}
		return fmt.Errorf("policy status does not report the missing jwtSign secret: %+v", status)
	})

	t.Send("invalid-jwt-sign.example.com", &testmatchers.HttpResponse{
		StatusCode: http.StatusInternalServerError,
		Body: gomega.And(
			gomega.ContainSubstring("backend authentication failed: jwtSign configuration is invalid"),
			gomega.Not(gomega.ContainSubstring(missingKeyRef)),
		),
	})
}
