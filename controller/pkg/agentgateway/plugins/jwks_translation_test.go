package plugins

import (
	"errors"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
)

type stubJWKSLookup struct {
	inline string
	err    error
}

func (s stubJWKSLookup) InlineForOwner(krt.HandlerContext, jwks.RemoteJwksOwner) (string, error) {
	return s.inline, s.err
}

func TestProcessJWTAuthenticationPolicyWhenLookupReturnsErrorOmitsRemoteProviderAndReturnsError(t *testing.T) {
	sentinel := errors.New("lookup failed")
	jwtAuth := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{{
			Issuer:    "issuer.example",
			Audiences: []string{"aud-a"},
			JWKS: agentgateway.JWKS{
				Remote: &agentgateway.RemoteJWKS{
					JwksPath: "/keys",
					BackendRef: gwv1.BackendObjectReference{
						Name: "jwks-backend",
					},
				},
			},
		}},
	}

	policy, err := processJWTAuthenticationPolicy(
		PolicyCtx{
			Krt:        krt.TestingDummyContext{},
			JWKSLookup: stubJWKSLookup{err: sentinel},
		},
		jwtAuth,
		nil,
		"default/test:jwt",
		types.NamespacedName{Namespace: "default", Name: "test"},
	)

	if err == nil || !errors.Is(err, sentinel) {
		t.Fatalf("expected lookup error, got %v", err)
	}
	if policy == nil {
		t.Fatal("expected policy to still be emitted")
	}
	jwtSpec := policy.GetTraffic().GetJwt()
	if jwtSpec == nil {
		t.Fatal("expected jwt spec")
	}
	if got := len(jwtSpec.Providers); got != 0 {
		t.Fatalf("expected remote provider to be omitted, got %d providers", got)
	}
	if jwtSpec.Mode != api.TrafficPolicySpec_JWT_STRICT {
		t.Fatalf("expected strict mode, got %v", jwtSpec.Mode)
	}
}

func TestTranslateMCPAuthenticationSpecWhenLookupReturnsErrorLeavesInlineEmptyAndReturnsError(t *testing.T) {
	sentinel := errors.New("lookup failed")
	authn := &agentgateway.MCPAuthentication{
		Issuer:    "issuer.example",
		Audiences: []string{"aud-a"},
		Mode:      agentgateway.JWTAuthenticationModePermissive,
		JWKS: agentgateway.RemoteJWKS{
			JwksPath: "/keys",
			BackendRef: gwv1.BackendObjectReference{
				Name: "jwks-backend",
			},
		},
	}

	spec, err := translateMCPAuthenticationSpec(
		PolicyCtx{
			Krt:        krt.TestingDummyContext{},
			JWKSLookup: stubJWKSLookup{err: sentinel},
		},
		types.NamespacedName{Namespace: "default", Name: "test"},
		authn,
	)

	if err == nil || !errors.Is(err, sentinel) {
		t.Fatalf("expected lookup error, got %v", err)
	}
	if spec == nil {
		t.Fatal("expected spec to still be emitted")
	}
	if spec.JwksInline != "" {
		t.Fatalf("expected jwks inline to be empty, got %q", spec.JwksInline)
	}
	if spec.Issuer != authn.Issuer {
		t.Fatalf("expected issuer %q, got %q", authn.Issuer, spec.Issuer)
	}
	if len(spec.Audiences) != 1 || spec.Audiences[0] != authn.Audiences[0] {
		t.Fatalf("expected audiences %v, got %v", authn.Audiences, spec.Audiences)
	}
	if spec.Mode != api.BackendPolicySpec_McpAuthentication_PERMISSIVE {
		t.Fatalf("expected permissive mode, got %v", spec.Mode)
	}
}
