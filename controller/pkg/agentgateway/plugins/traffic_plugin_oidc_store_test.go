package plugins_test

import (
	"testing"

	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	k8stypes "k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
)

const testOIDCJWKSInline = `{"keys":[{"kid":"k1"}]}`

func TestTranslateAgentgatewayPolicyTranslatesIssuerOAuth2FromControllerStore(t *testing.T) {
	backendRef := &gwv1.BackendObjectReference{
		Name: "oauth2-discovery",
		Kind: ptr.Of(gwv1.Kind("Service")),
		Port: ptr.Of(gwv1.PortNumber(8443)),
	}
	oauth2 := testOAuth2DiscoveryConfig()
	oauth2.BackendRef = backendRef
	policy := testGatewayOAuth2PolicyForConfig("oauth2-controller-store", oauth2)

	provider := oidc.StoredProvider{
		Issuer:                            "https://issuer.example.com",
		AuthorizationEndpoint:             "https://issuer.example.com/oauth/authorize",
		TokenEndpoint:                     "https://issuer.example.com/oauth/token",
		JwksURI:                           "https://issuer.example.com/oauth/jwks",
		EndSessionEndpoint:                "https://issuer.example.com/logout",
		TokenEndpointAuthMethodsSupported: []string{"client_secret_post"},
	}
	resolvedJwksURI := "https://oauth2-discovery.default.svc.cluster.local:8443/oauth/jwks"

	status, output := testTranslateGatewayOAuth2PolicyWithInputs(t, policy,
		testService("oauth2-discovery", "default", 8443),
		testOIDCProviderConfigMap(t, policy.Namespace, string(*oauth2.Issuer), backendRef, provider),
		testJWKSConfigMap(t, resolvedJwksURI, testOIDCJWKSInline),
	)
	if len(output) != 1 {
		t.Fatalf("expected 1 translated policy, got %d", len(output))
	}
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonValid) {
		t.Fatalf("expected valid reason, got %q", accepted.Reason)
	}

	spec := output[0].Policy.GetTraffic().GetOauth2()
	if spec == nil {
		t.Fatalf("expected translated oauth2 spec")
	}
	if got, want := spec.GetOidcIssuer(), provider.Issuer; got != want {
		t.Fatalf("unexpected oidc issuer: got %q want %q", got, want)
	}
	if got, want := spec.GetProviderId(), provider.Issuer; got != want {
		t.Fatalf("unexpected provider id: got %q want %q", got, want)
	}
	if got, want := spec.GetAuthorizationEndpoint(), provider.AuthorizationEndpoint; got != want {
		t.Fatalf("unexpected authorization endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetTokenEndpoint(), provider.TokenEndpoint; got != want {
		t.Fatalf("unexpected token endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetEndSessionEndpoint(), provider.EndSessionEndpoint; got != want {
		t.Fatalf("unexpected end session endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetJwksInline(), testOIDCJWKSInline; got != want {
		t.Fatalf("unexpected jwks inline: got %q want %q", got, want)
	}
	if got := spec.GetTokenEndpointAuthMethodsSupported(); len(got) != 1 || got[0] != "client_secret_post" {
		t.Fatalf("unexpected token endpoint auth methods: %v", got)
	}
	if backend := spec.GetProviderBackend(); backend == nil {
		t.Fatalf("expected translated provider backend")
	} else {
		if got, want := backend.GetService().Hostname, "oauth2-discovery.default.svc.cluster.local"; got != want {
			t.Fatalf("unexpected provider backend hostname: got %q want %q", got, want)
		}
		if got, want := backend.GetPort(), uint32(8443); got != want {
			t.Fatalf("unexpected provider backend port: got %d want %d", got, want)
		}
	}
}

func TestTranslateAgentgatewayPolicyTranslatesJWTOIDCProviderFromControllerStore(t *testing.T) {
	policy := testGatewayJWTPolicy("jwt-controller-store", k8stypes.UID("uid-jwt-controller-store"))
	policy.Spec.Traffic.JWTAuthentication = &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{
			{
				Issuer: "https://issuer.example.com",
				JWKS: agentgateway.JWKS{
					OIDC: &agentgateway.OIDCJWKS{},
				},
			},
		},
	}

	provider := oidc.StoredProvider{
		Issuer:                "https://issuer.example.com",
		AuthorizationEndpoint: "https://issuer.example.com/oauth/authorize",
		TokenEndpoint:         "https://issuer.example.com/oauth/token",
		JwksURI:               "https://issuer.example.com/oauth/jwks",
	}

	ctx := testutils.BuildMockPolicyContext(t, []any{
		testGateway(),
		policy,
		testOIDCProviderConfigMap(t, policy.Namespace, provider.Issuer, nil, provider),
		testJWKSConfigMap(t, provider.JwksURI, testOIDCJWKSInline),
	})
	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, policy, ctx.Collections)
	if len(output) != 1 {
		t.Fatalf("expected 1 translated policy, got %d", len(output))
	}
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonValid) {
		t.Fatalf("expected valid reason, got %q", accepted.Reason)
	}

	spec := output[0].Policy.GetTraffic().GetJwt()
	if spec == nil {
		t.Fatalf("expected translated jwt spec")
	}
	if len(spec.GetProviders()) != 1 {
		t.Fatalf("expected 1 provider, got %d", len(spec.GetProviders()))
	}
	if got, want := spec.GetProviders()[0].GetInline(), testOIDCJWKSInline; got != want {
		t.Fatalf("unexpected inline jwks: got %q want %q", got, want)
	}
}

func TestResolveOIDCJWKSSourceUsesStoredProviderConfigMap(t *testing.T) {
	provider := oidc.StoredProvider{
		Issuer:                "https://issuer.example.com",
		AuthorizationEndpoint: "https://issuer.example.com/oauth/authorize",
		TokenEndpoint:         "https://issuer.example.com/oauth/token",
		JwksURI:               "https://issuer.example.com/oauth/jwks",
	}
	ctx := testutils.BuildMockPolicyContext(t, []any{
		testOIDCProviderConfigMap(t, "default", provider.Issuer, nil, provider),
	})

	source, err := plugins.ResolveOIDCJWKSSource(ctx, "jwt-policy", "default", provider.Issuer, nil)
	if err != nil {
		t.Fatalf("unexpected error resolving oidc jwks source: %v", err)
	}
	if got, want := source.JwksURL, provider.JwksURI; got != want {
		t.Fatalf("unexpected jwks url: got %q want %q", got, want)
	}
	if got := source.HostOverride; got != "" {
		t.Fatalf("did not expect host override, got %q", got)
	}
	if source.TlsConfig != nil {
		t.Fatalf("did not expect controller-side tls config without a backend override")
	}
}

func testOIDCProviderConfigMap(
	t *testing.T,
	policyNamespace string,
	issuer string,
	backendRef *gwv1.BackendObjectReference,
	provider oidc.StoredProvider,
) *corev1.ConfigMap {
	t.Helper()

	resourceKey, err := oidc.CanonicalSourceKey(issuer, policyNamespace, backendRef)
	if err != nil {
		t.Fatalf("failed building oidc provider resource key: %v", err)
	}
	provider.ResourceKey = resourceKey
	if provider.Issuer == "" {
		provider.Issuer = issuer
	}

	name := oidc.ProviderConfigMapNamespacedName(resourceKey)
	if name == nil {
		t.Fatal("oidc provider store naming function is not initialized")
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name.Name,
			Namespace: name.Namespace,
			Labels:    oidc.StoreConfigMapLabel(oidc.DefaultStorePrefix),
		},
		Data: map[string]string{},
	}
	if err := oidc.SetProviderInConfigMap(cm, provider); err != nil {
		t.Fatalf("failed storing oidc provider in ConfigMap: %v", err)
	}
	return cm
}

func testJWKSConfigMap(t *testing.T, jwksURI string, inline string) *corev1.ConfigMap {
	t.Helper()

	name := jwks.JwksConfigMapNamespacedName(jwks.DefaultJwksStorePrefix, "agentgateway-system", jwksURI)
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name.Name,
			Namespace: name.Namespace,
			Labels:    jwks.JwksStoreConfigMapLabel(jwks.DefaultJwksStorePrefix),
		},
		Data: map[string]string{},
	}
	if err := jwks.SetJwksInConfigMap(cm, jwksURI, inline); err != nil {
		t.Fatalf("failed storing jwks in ConfigMap: %v", err)
	}
	return cm
}

func testService(name, namespace string, port int32) *corev1.Service {
	return &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: namespace,
		},
		Spec: corev1.ServiceSpec{
			Ports: []corev1.ServicePort{
				{Port: port},
			},
		},
	}
}
