package plugins_test

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/ptr"
	k8stypes "k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestTranslateAgentgatewayPolicyRejectsUnsupportedJWTOIDCBackendRefKind(t *testing.T) {
	policy := testGatewayJWTPolicyWithOIDCBackend("jwt-unsupported-backend-kind", &gwv1.BackendObjectReference{
		Group: ptr.Of(gwv1.Group(wellknown.InferencePoolGVK.Group)),
		Kind:  ptr.Of(gwv1.Kind(wellknown.InferencePoolGVK.Kind)),
		Name:  "test-pool",
		Port:  ptr.Of(gwv1.PortNumber(8443)),
	})

	ctx := testutils.BuildMockPolicyContext(t, []any{testGateway(), policy})
	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, policy, ctx.Collections)
	_ = output

	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonPartiallyValid) {
		t.Fatalf("expected partially valid reason, got %q", accepted.Reason)
	}
	if !strings.Contains(accepted.Message, "jwt oidc provider backend ref only supports Service and AgentgatewayBackend kinds") {
		t.Fatalf("unexpected accepted message: %q", accepted.Message)
	}
}

func testGatewayJWTPolicyWithOIDCBackend(name string, backendRef *gwv1.BackendObjectReference) *agentgateway.AgentgatewayPolicy {
	policy := testGatewayJWTPolicy(name, k8stypes.UID("uid-"+name))
	policy.Spec.Traffic.JWTAuthentication = &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{
			{
				Issuer: "https://issuer.example.com",
				JWKS: agentgateway.JWKS{
					OIDC: &agentgateway.OIDCJWKS{
						BackendRef: backendRef,
					},
				},
			},
		},
	}
	return policy
}
