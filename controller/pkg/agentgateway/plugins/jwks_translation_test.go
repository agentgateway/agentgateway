package plugins

import (
	"encoding/json"
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

func longStringPtr(s string) *agentgateway.LongString {
	v := s
	return &v
}

func TestProcessJWTAuthenticationPolicyWhenLookupReturnsErrorPreservesRemoteProviderAndReturnsError(t *testing.T) {
	sentinel := errors.New("lookup failed")
	jwtAuth := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{{
			Issuer:    "issuer.example",
			Audiences: []string{"aud-a"},
			JWKS: &agentgateway.JWKS{
				Remote: &agentgateway.RemoteJWKS{
					JwksPath: longStringPtr("/keys"),
					BackendRef: &gwv1.BackendObjectReference{
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
	if got := len(jwtSpec.Providers); got != 1 {
		t.Fatalf("expected remote provider to be preserved, got %d providers", got)
	}
	provider := jwtSpec.Providers[0]
	if provider.GetInline() != `{"keys":[]}` {
		t.Fatalf("expected empty key set, got %q", provider.GetInline())
	}
	if provider.Issuer != jwtAuth.Providers[0].Issuer || len(provider.Audiences) != 1 || provider.Audiences[0] != "aud-a" {
		t.Fatalf("expected issuer and audiences to be preserved, got %v", provider)
	}
	if jwtSpec.Mode != api.TrafficPolicySpec_JWT_STRICT {
		t.Fatalf("expected strict mode, got %v", jwtSpec.Mode)
	}
}

func TestProcessJWKSInvalidInline(t *testing.T) {
	inlineBad := agentgateway.LongString(`{"keys":[{"e":"AQAB","kid":"3161","kty":"RSB","n":"tmzcODUF5T9p"}]}`)
	jwtAuth := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{{
			Issuer: "cool-issuer.corp",
			JWKS: &agentgateway.JWKS{
				Inline: &inlineBad,
			},
		}},
	}
	policy, err := processJWTAuthenticationPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		jwtAuth,
		nil,
		"default/test:jwt",
		types.NamespacedName{Namespace: "default", Name: "test"},
	)

	if err == nil {
		t.Fatal("expected error for invalid inline JWKS, got nil")
	}
	if got := len(policy.GetTraffic().GetJwt().GetProviders()); got != 1 {
		t.Fatalf("expected the bad provider to be dropped (0 providers), got %d", got)
	}
}

func TestProcessJWTAuthenticationPolicyTranslatesEmptyRequiredClaims(t *testing.T) {
	inline := agentgateway.LongString(`{"keys":[]}`)
	jwtAuth := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{{
			Issuer: "issuer.example",
			JWKS:   &agentgateway.JWKS{Inline: &inline},
			Validation: &agentgateway.JWTValidationOptions{
				RequiredClaims: new([]agentgateway.JWTClaim{}),
			},
		}},
	}

	// Exercise the typed client's JSON boundary before translating the policy.
	wire, err := json.Marshal(jwtAuth)
	if err != nil {
		t.Fatal(err)
	}
	jwtAuth = &agentgateway.JWTAuthentication{}
	if err := json.Unmarshal(wire, jwtAuth); err != nil {
		t.Fatal(err)
	}

	policy, err := processJWTAuthenticationPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		jwtAuth,
		nil,
		"default/test:jwt",
		types.NamespacedName{Namespace: "default", Name: "test"},
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	got := policy.GetTraffic().GetJwt().GetProviders()[0].GetJwtValidationOptions()
	if got == nil {
		t.Fatal("expected jwt validation options to be set")
	}
	if len(got.GetRequiredClaims()) != 0 {
		t.Fatalf("expected empty required claims, got %v", got.GetRequiredClaims())
	}
}

func TestProcessJWTAuthenticationPolicyDefaultsRequiredClaimsWhenOptionsEmpty(t *testing.T) {
	inline := agentgateway.LongString(`{"keys":[]}`)
	jwtAuth := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{{
			Issuer:     "issuer.example",
			JWKS:       &agentgateway.JWKS{Inline: &inline},
			Validation: &agentgateway.JWTValidationOptions{},
		}},
	}

	policy, err := processJWTAuthenticationPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		jwtAuth,
		nil,
		"default/test:jwt",
		types.NamespacedName{Namespace: "default", Name: "test"},
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	got := policy.GetTraffic().GetJwt().GetProviders()[0].GetJwtValidationOptions().GetRequiredClaims()
	if len(got) != 1 || got[0] != "exp" {
		t.Fatalf("expected default required claims [exp], got %v", got)
	}
}

func TestProcessJWTAuthenticationPolicyOmitsValidationOptionsWhenUnset(t *testing.T) {
	inline := agentgateway.LongString(`{"keys":[]}`)
	jwtAuth := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{{
			Issuer: "issuer.example",
			JWKS:   &agentgateway.JWKS{Inline: &inline},
		}},
	}

	policy, err := processJWTAuthenticationPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		jwtAuth,
		nil,
		"default/test:jwt",
		types.NamespacedName{Namespace: "default", Name: "test"},
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got := policy.GetTraffic().GetJwt().GetProviders()[0].GetJwtValidationOptions(); got != nil {
		t.Fatalf("expected unset jwt validation options, got %#v", got)
	}
}

func TestTranslateMCPAuthenticationSpecTranslatesEmptyRequiredClaims(t *testing.T) {
	authn := &agentgateway.MCPAuthentication{
		Issuer: "issuer.example",
		JWKS: &agentgateway.RemoteJWKS{
			JwksPath: longStringPtr("/keys"),
			BackendRef: &gwv1.BackendObjectReference{
				Name: "jwks-backend",
			},
		},
		Validation: &agentgateway.JWTValidationOptions{
			RequiredClaims: new([]agentgateway.JWTClaim{}),
		},
	}

	// Exercise the typed client's JSON boundary before translating the policy.
	wire, err := json.Marshal(authn)
	if err != nil {
		t.Fatal(err)
	}
	authn = &agentgateway.MCPAuthentication{}
	if err := json.Unmarshal(wire, authn); err != nil {
		t.Fatal(err)
	}

	spec, err := translateMCPAuthenticationSpec(
		PolicyCtx{
			Krt:        krt.TestingDummyContext{},
			JWKSLookup: stubJWKSLookup{inline: `{"keys":[]}`},
		},
		types.NamespacedName{Namespace: "default", Name: "test"},
		authn,
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	got := spec.GetJwtValidationOptions()
	if got == nil {
		t.Fatal("expected jwt validation options to be set")
	}
	if len(got.GetRequiredClaims()) != 0 {
		t.Fatalf("expected empty required claims, got %v", got.GetRequiredClaims())
	}
}

func TestTranslateMCPAuthenticationSpecWhenLookupReturnsErrorEmitsEmptyKeySetAndReturnsError(t *testing.T) {
	sentinel := errors.New("lookup failed")
	authn := &agentgateway.MCPAuthentication{
		Issuer:    "issuer.example",
		Audiences: []string{"aud-a"},
		Mode:      agentgateway.JWTAuthenticationModePermissive,
		JWKS: &agentgateway.RemoteJWKS{
			JwksPath: longStringPtr("/keys"),
			BackendRef: &gwv1.BackendObjectReference{
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
	if spec.JwksInline != `{"keys":[]}` {
		t.Fatalf("expected empty key set, got %q", spec.JwksInline)
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
