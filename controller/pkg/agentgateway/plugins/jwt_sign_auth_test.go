package plugins

import (
	"strings"
	"testing"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

// translateJwtSignSigningAlg is documented to reject unrecognized alg values
// (guarding against version skew, since the CRD enum otherwise prevents this)
// rather than falling back to a default. buildJwtSignAuthPolicy must honor
// that rejection instead of still emitting a policy with the wrong alg.
func TestJwtSignRejectsUnsupportedSigningAlg(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)
	badAlg := agentgateway.OAuthPrivateKeyJWTSigningAlgorithm("HS256")
	policy := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default"},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					Auth: &agentgateway.BackendAuth{
						JwtSign: &agentgateway.JwtSignAuth{
							SigningKeyRef: agentgateway.LocalSecretObjectRef{Name: "jwt-sign-secret"},
							Alg:           &badAlg,
							Claims:        map[string]string{"iss": "acct.user"},
						},
					},
				},
			},
		},
	}

	p, err := translateBackendAuth(ctx, policy, "default/jwt-sign")
	if err == nil || !strings.Contains(err.Error(), "unsupported jwtSign signing algorithm") {
		t.Fatalf("translateBackendAuth() error = %v, want unsupported signing algorithm error", err)
	}
	if p != nil {
		t.Fatalf("translateBackendAuth() policy = %v, want nil policy on unsupported alg", p)
	}
}
