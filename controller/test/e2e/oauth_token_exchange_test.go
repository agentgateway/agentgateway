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
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/gomega/transforms"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
	"github.com/agentgateway/agentgateway/controller/test/testutils/testjwt"
)

const invalidOAuthTokenType = agentgateway.OAuthTokenType("not-a-token-type")

func TestOAuthTokenExchange(tt *testing.T) {
	t := New(tt)
	t.Apply(manifest("oauth", "routes.yaml"))

	t.HTTPRouteAccepted("cross-app-access", base.Namespace)
	t.HTTPRouteAccepted("oauth-token-exchange", base.Namespace)
	t.HTTPRouteAccepted("oauth-jwt-subject", base.Namespace)
	t.HTTPRouteAccepted("oauth-jwt-bearer", base.Namespace)
	t.HTTPRouteAccepted("invalid-oauth-token-exchange", base.Namespace)

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

	t.Run("InvalidTokenTypeUpdate", func(t base.Test) {
		const policyName = "oauth-token-exchange"
		policyKey := types.NamespacedName{Name: policyName, Namespace: base.Namespace}
		policy := &agentgateway.AgentgatewayPolicy{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), policyKey, policy); err != nil {
			t.Fatalf("failed to get valid OAuth policy: %v", err)
		}
		validAuth := policy.Spec.Backend.Auth.OAuthTokenExchange.DeepCopy()
		testutils.Cleanup(t, func() {
			updateOAuthTokenExchange(t, policyKey, func(auth *agentgateway.OAuthTokenExchange) {
				*auth = *validAuth.DeepCopy()
			})
			waitForOAuthPolicyReason(t, policyKey, string(agentgateway.PolicyReasonValid), "")
		})

		tests := []struct {
			name        string
			wantMessage string
			mutate      func(*agentgateway.OAuthTokenExchange)
		}{
			{
				name:        "SubjectToken",
				wantMessage: "oauth subjectToken tokenType",
				mutate: func(auth *agentgateway.OAuthTokenExchange) {
					auth.SubjectToken = &agentgateway.OAuthTokenSpec{TokenType: new(invalidOAuthTokenType)}
				},
			},
			{
				name:        "ActorToken",
				wantMessage: "oauth actorToken tokenType",
				mutate: func(auth *agentgateway.OAuthTokenExchange) {
					auth.ActorToken.TokenType = new(invalidOAuthTokenType)
				},
			},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t base.Test) {
				updateOAuthTokenExchange(t, policyKey, func(auth *agentgateway.OAuthTokenExchange) {
					*auth = *validAuth.DeepCopy()
				})
				waitForOAuthPolicyReason(t, policyKey, string(agentgateway.PolicyReasonValid), "")
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

				updateOAuthTokenExchange(t, policyKey, tt.mutate)
				waitForOAuthPolicyReason(t, policyKey, string(agentgateway.PolicyReasonPartiallyValid), tt.wantMessage)
				t.Send("oauth-token-exchange.com",
					&testmatchers.HttpResponse{
						StatusCode: http.StatusInternalServerError,
						Body: gomega.And(
							gomega.ContainSubstring("OAuth token exchange configuration is invalid"),
							gomega.Not(gomega.ContainSubstring(string(invalidOAuthTokenType))),
						),
					},
					curl.WithHeader("Authorization", "Bearer subject-token"),
					curl.WithHeader("X-Actor-Token", "actor-token"),
					curl.WithHeader("X-Tenant", "tenant-a"),
				)
			})
		}
	})

	t.Run("InvalidConfiguration", func(t base.Test) {
		const wantMessage = "oauth subjectToken tokenType"
		retry.UntilSuccessOrFail(t, func() error {
			policy := &agentgateway.AgentgatewayPolicy{}
			if err := t.E2EClusterContext().ControllerClient.Get(
				t.E2EContext(),
				types.NamespacedName{Name: "invalid-oauth-token-exchange", Namespace: base.Namespace},
				policy,
			); err != nil {
				return err
			}
			for _, ancestor := range policy.Status.Ancestors {
				for _, condition := range ancestor.Conditions {
					if condition.Type == "Accepted" &&
						condition.Status == metav1.ConditionTrue &&
						condition.Reason == "PartiallyValid" &&
						strings.Contains(condition.Message, wantMessage) {
						return nil
					}
				}
			}
			return fmt.Errorf("policy status does not report Accepted=True/PartiallyValid for %s", wantMessage)
		})

		t.Send("invalid-oauth-token-exchange.com",
			&testmatchers.HttpResponse{
				StatusCode: http.StatusInternalServerError,
				Body: gomega.And(
					gomega.ContainSubstring("OAuth token exchange configuration is invalid"),
					gomega.Not(gomega.ContainSubstring(string(invalidOAuthTokenType))),
				),
			},
			curl.WithHeader("Authorization", "Bearer subject-token"),
		)
	})

	assertions.EventuallyAgwPolicyCondition(t, "oauth-jwt-subject-auth", base.Namespace, "Accepted", metav1.ConditionTrue)
	assertions.EventuallyAgwPolicyCondition(t, "oauth-jwt-subject", base.Namespace, "Accepted", metav1.ConditionTrue)
	t.Run("ValidatedJWTSubject", func(t base.Test) {
		t.Send("oauth-jwt-subject.com",
			&testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.HaveKeyWithValue("Authorization", "Bearer jwt-subject-token-exchange-access"),
				),
			},
			curl.WithHeader("Authorization", "Bearer "+testjwt.OrgOneJWT),
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

func updateOAuthTokenExchange(
	t base.Test,
	policyKey types.NamespacedName,
	mutate func(*agentgateway.OAuthTokenExchange),
) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		policy := &agentgateway.AgentgatewayPolicy{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), policyKey, policy); err != nil {
			return err
		}
		mutate(policy.Spec.Backend.Auth.OAuthTokenExchange)
		return t.E2EClusterContext().ControllerClient.Update(t.E2EContext(), policy)
	})
}

func waitForOAuthPolicyReason(
	t base.Test,
	policyKey types.NamespacedName,
	reason string,
	message string,
) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		policy := &agentgateway.AgentgatewayPolicy{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), policyKey, policy); err != nil {
			return err
		}
		for _, ancestor := range policy.Status.Ancestors {
			for _, condition := range ancestor.Conditions {
				if condition.Type == string(agentgateway.PolicyConditionAccepted) &&
					condition.Status == metav1.ConditionTrue &&
					condition.Reason == reason &&
					strings.Contains(condition.Message, message) {
					return nil
				}
			}
		}
		return fmt.Errorf("policy status does not report Accepted=True/%s containing %q", reason, message)
	})
}
