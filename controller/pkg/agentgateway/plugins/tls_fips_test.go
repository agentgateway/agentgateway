package plugins

import (
	"crypto/fips140"
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestValidateFIPSTLSOptions(t *testing.T) {
	for _, tt := range []struct {
		suite agentgateway.CipherSuite
		valid bool
	}{
		{agentgateway.CipherSuiteTLS13_AES_256_GCM_SHA384, true},
		{agentgateway.CipherSuiteTLS13_AES_128_GCM_SHA256, true},
		{agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384, true},
		{agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256, true},
		{agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384, true},
		{agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, true},
		{agentgateway.CipherSuiteTLS13_CHACHA20_POLY1305_SHA256, false},
		{agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256, false},
		{agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256, false},
	} {
		t.Run(string(tt.suite), func(t *testing.T) {
			err := validateFIPSTLSOptions([]agentgateway.CipherSuite{tt.suite}, nil)
			if tt.valid {
				require.NoError(t, err)
			} else {
				require.ErrorContains(t, err, string(tt.suite))
			}
		})
	}
	for _, tt := range []struct {
		group agentgateway.KeyExchangeGroup
		valid bool
	}{
		{agentgateway.KeyExchangeGroupP256, true},
		{agentgateway.KeyExchangeGroupP384, true},
		{agentgateway.KeyExchangeGroupX25519MLKEM768, true},
		{agentgateway.KeyExchangeGroupX25519, false},
	} {
		t.Run(string(tt.group), func(t *testing.T) {
			err := validateFIPSTLSOptions(nil, []agentgateway.KeyExchangeGroup{tt.group})
			if tt.valid {
				require.NoError(t, err)
			} else {
				require.ErrorContains(t, err, string(tt.group))
			}
		})
	}
	t.Run("defaults", func(t *testing.T) {
		require.NoError(t, validateFIPSTLSOptions(nil, nil))
	})
	t.Run("mixed options", func(t *testing.T) {
		err := validateFIPSTLSOptions(
			[]agentgateway.CipherSuite{agentgateway.CipherSuiteTLS13_AES_128_GCM_SHA256, agentgateway.CipherSuiteTLS13_CHACHA20_POLY1305_SHA256},
			[]agentgateway.KeyExchangeGroup{agentgateway.KeyExchangeGroupP256, agentgateway.KeyExchangeGroupX25519, agentgateway.KeyExchangeGroupX25519MLKEM768},
		)
		require.EqualError(t, err, "TLS configuration is not permitted in FIPS mode: non-approved cipher suites [TLS13_CHACHA20_POLY1305_SHA256], non-approved key exchange groups [X25519]")
	})
}

func TestFIPSTLSPolicyStatus(t *testing.T) {
	t.Logf("FIPS mode: %t", fips140.Enabled())
	approvedGroups := []agentgateway.KeyExchangeGroup{
		agentgateway.KeyExchangeGroupP256,
		agentgateway.KeyExchangeGroupP384,
		agentgateway.KeyExchangeGroupX25519MLKEM768,
	}
	approvedSuites := []agentgateway.CipherSuite{
		agentgateway.CipherSuiteTLS13_AES_256_GCM_SHA384,
		agentgateway.CipherSuiteTLS13_AES_128_GCM_SHA256,
		agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
		agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
		agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
		agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
	}
	tests := []struct {
		name     string
		frontend *agentgateway.FrontendTLS
		backend  *agentgateway.BackendTLS
		rejected []string
	}{
		{
			name:     "frontend defaults",
			frontend: &agentgateway.FrontendTLS{},
		},
		{
			name:    "backend defaults",
			backend: &agentgateway.BackendTLS{},
		},
		{
			name: "frontend approved options",
			frontend: &agentgateway.FrontendTLS{
				CipherSuites: approvedSuites, KeyExchangeGroups: approvedGroups,
			},
		},
		{
			name:    "backend approved groups",
			backend: &agentgateway.BackendTLS{KeyExchangeGroups: approvedGroups},
		},
		{
			name:     "frontend TLS 1.3 ChaCha20",
			frontend: &agentgateway.FrontendTLS{CipherSuites: []agentgateway.CipherSuite{agentgateway.CipherSuiteTLS13_CHACHA20_POLY1305_SHA256}},
			rejected: []string{"TLS13_CHACHA20_POLY1305_SHA256"},
		},
		{
			name:     "frontend ECDHE ECDSA ChaCha20",
			frontend: &agentgateway.FrontendTLS{CipherSuites: []agentgateway.CipherSuite{agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256}},
			rejected: []string{"TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"},
		},
		{
			name:     "frontend ECDHE RSA ChaCha20",
			frontend: &agentgateway.FrontendTLS{CipherSuites: []agentgateway.CipherSuite{agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256}},
			rejected: []string{"TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"},
		},
		{
			name:     "frontend X25519",
			frontend: &agentgateway.FrontendTLS{KeyExchangeGroups: []agentgateway.KeyExchangeGroup{agentgateway.KeyExchangeGroupX25519}},
			rejected: []string{"X25519"},
		},
		{
			name:     "backend X25519",
			backend:  &agentgateway.BackendTLS{KeyExchangeGroups: []agentgateway.KeyExchangeGroup{agentgateway.KeyExchangeGroupX25519}},
			rejected: []string{"X25519"},
		},
		{
			name: "mixed approved and non-approved options",
			frontend: &agentgateway.FrontendTLS{
				CipherSuites:      append(approvedSuites, agentgateway.CipherSuiteTLS13_CHACHA20_POLY1305_SHA256),
				KeyExchangeGroups: append(approvedGroups, agentgateway.KeyExchangeGroupX25519),
			},
			backend:  &agentgateway.BackendTLS{KeyExchangeGroups: []agentgateway.KeyExchangeGroup{agentgateway.KeyExchangeGroupX25519}},
			rejected: []string{"TLS13_CHACHA20_POLY1305_SHA256", "X25519"},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			collections, references, policy := fipsTLSPolicyContext(t)
			if tt.frontend != nil {
				policy.Spec.Frontend = &agentgateway.Frontend{TLS: tt.frontend}
			}
			if tt.backend != nil {
				policy.Spec.Backend = &agentgateway.BackendFull{TLS: tt.backend}
			}
			status, translated := TranslateAgentgatewayPolicy(krt.TestingDummyContext{}, policy, collections, references, nil, nil, nil, nil)
			require.Len(t, status.Ancestors, 1)
			accepted := meta.FindStatusCondition(status.Ancestors[0].Conditions, agentgateway.PolicyConditionAccepted)
			require.NotNil(t, accepted)
			require.Equal(t, policy.Generation, accepted.ObservedGeneration)
			attached := meta.FindStatusCondition(status.Ancestors[0].Conditions, agentgateway.PolicyConditionAttached)
			require.NotNil(t, attached)
			require.Equal(t, metav1.ConditionTrue, attached.Status)
			require.Equal(t, agentgateway.PolicyReasonAttached, attached.Reason)
			if fips140.Enabled() && len(tt.rejected) > 0 {
				require.Equal(t, metav1.ConditionFalse, accepted.Status)
				require.Equal(t, agentgateway.PolicyReasonInvalid, accepted.Reason)
				require.NotContains(t, accepted.Message, "\n")
				for _, value := range tt.rejected {
					require.Contains(t, accepted.Message, value)
				}
				if tt.frontend != nil {
					require.Contains(t, accepted.Message, "frontend.tls:")
				}
				if tt.backend != nil {
					require.Contains(t, accepted.Message, "backend.tls:")
				}
				if tt.frontend != nil && tt.backend != nil {
					require.Contains(t, accepted.Message, "; backend.tls:")
				}
			} else {
				require.Equal(t, metav1.ConditionTrue, accepted.Status)
				require.Equal(t, agentgateway.PolicyReasonValid, accepted.Reason)
			}

			var frontend *api.FrontendPolicySpec_TLS
			var backend *api.BackendPolicySpec_BackendTLS
			for _, p := range translated {
				if tls := p.Policy.GetFrontend().GetTls(); tls != nil {
					frontend = tls
				}
				if tls := p.Policy.GetBackend().GetBackendTls(); tls != nil {
					backend = tls
				}
			}
			if tt.frontend != nil {
				require.NotNil(t, frontend)
				require.Len(t, frontend.CipherSuites, len(tt.frontend.CipherSuites))
				require.Equal(t, convertTLSKeyExchangeGroups(tt.frontend.KeyExchangeGroups), frontend.KeyExchangeGroups)
			}
			if tt.backend != nil {
				require.NotNil(t, backend)
				require.Equal(t, convertTLSKeyExchangeGroups(tt.backend.KeyExchangeGroups), backend.KeyExchangeGroups)
			}
		})
	}
}

func TestFIPSTLSPolicyDiagnostics(t *testing.T) {
	policy := &agentgateway.AgentgatewayPolicy{
		Name: "policy", Namespace: "default",
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{TLS: &agentgateway.BackendTLS{
				KeyExchangeGroups: []agentgateway.KeyExchangeGroup{agentgateway.KeyExchangeGroupX25519},
			}},
		},
	}
	policies, err := TranslatePolicyToAgw(PolicyCtx{}, policy)
	require.Len(t, policies, 1)
	conditions := PolicyConditionMap(err, len(policies) > 0)
	accepted := conditions[agentgateway.PolicyConditionAccepted]
	if fips140.Enabled() {
		require.ErrorContains(t, err, "backend.tls:")
		require.Equal(t, metav1.ConditionFalse, accepted.Status)
		require.Equal(t, agentgateway.PolicyReasonInvalid, accepted.Reason)
	} else {
		require.NoError(t, err)
		require.Equal(t, metav1.ConditionTrue, accepted.Status)
	}
	require.Equal(t, metav1.ConditionTrue, conditions[agentgateway.PolicyConditionAttached].Status)
}

func TestFIPSTLSPolicyStatusUpdate(t *testing.T) {
	for _, backend := range []bool{false, true} {
		name := "frontend"
		if backend {
			name = "backend"
		}
		t.Run(name, func(t *testing.T) {
			collections, references, policy := fipsTLSPolicyContext(t)
			policy.Spec.Frontend = &agentgateway.Frontend{HTTP: &agentgateway.FrontendHTTP{MaxConcurrentRequests: new(int32(10))}}
			for i, group := range []agentgateway.KeyExchangeGroup{
				agentgateway.KeyExchangeGroupP256,
				agentgateway.KeyExchangeGroupX25519,
				agentgateway.KeyExchangeGroupP384,
			} {
				policy.Generation = int64(i + 1)
				if backend {
					policy.Spec.Backend = &agentgateway.BackendFull{TLS: &agentgateway.BackendTLS{KeyExchangeGroups: []agentgateway.KeyExchangeGroup{group}}}
				} else {
					policy.Spec.Frontend.TLS = &agentgateway.FrontendTLS{KeyExchangeGroups: []agentgateway.KeyExchangeGroup{group}}
				}
				status, translated := TranslateAgentgatewayPolicy(krt.TestingDummyContext{}, policy, collections, references, nil, nil, nil, nil)
				require.Len(t, translated, 2)
				require.Len(t, status.Ancestors, 1)
				accepted := meta.FindStatusCondition(status.Ancestors[0].Conditions, agentgateway.PolicyConditionAccepted)
				require.NotNil(t, accepted)
				require.Equal(t, policy.Generation, accepted.ObservedGeneration)
				attached := meta.FindStatusCondition(status.Ancestors[0].Conditions, agentgateway.PolicyConditionAttached)
				require.NotNil(t, attached)
				require.Equal(t, metav1.ConditionTrue, attached.Status)
				require.Equal(t, agentgateway.PolicyReasonAttached, attached.Reason)
				if fips140.Enabled() && group == agentgateway.KeyExchangeGroupX25519 {
					require.Equal(t, metav1.ConditionFalse, accepted.Status)
					require.Equal(t, agentgateway.PolicyReasonInvalid, accepted.Reason)
					require.Contains(t, accepted.Message, "X25519")
					require.Contains(t, accepted.Message, name+".tls:")
				} else {
					require.Equal(t, metav1.ConditionTrue, accepted.Status)
					require.Equal(t, agentgateway.PolicyReasonValid, accepted.Reason)
					require.NotContains(t, accepted.Message, "X25519")
				}
				policy.Status = *status
			}
		})
	}
}

func TestFIPSTLSGuardrailInlinePolicies(t *testing.T) {
	connection := agentgateway.BackendConnectionPolicy{
		TLS: &agentgateway.BackendTLS{KeyExchangeGroups: []agentgateway.KeyExchangeGroup{agentgateway.KeyExchangeGroupX25519}},
		Tunnel: &agentgateway.BackendTunnel{
			PolicyBackendEndpoint: agentgateway.PolicyBackendEndpoint{URL: new("http://proxy.example.com:8080")},
			Mode:                  agentgateway.BackendTunnelModeConnect,
		},
	}
	ctx := backendTLSContext(t, nil, nil)
	tests := []struct {
		name      string
		translate func() []*api.BackendPolicySpec
	}{
		{
			name: "moderation",
			translate: func() []*api.BackendPolicySpec {
				return processModeration(ctx, "default", &agentgateway.OpenAIModeration{
					Policies: &agentgateway.OpenAIModerationPolicy{
						BackendConnectionPolicy: connection,
						Auth:                    &agentgateway.OpenAIModerationAuth{InlineKey: new("test-key")},
					},
				}).InlinePolicies
			},
		},
		{
			name: "bedrock guardrails",
			translate: func() []*api.BackendPolicySpec {
				return processBedrockGuardrails(ctx, "default", &agentgateway.BedrockGuardrails{
					Policies: &agentgateway.BedrockGuardrailsPolicy{
						BackendConnectionPolicy: connection,
						Auth:                    &agentgateway.BedrockGuardrailsAuth{InlineKey: new("test-key")},
					},
				}).InlinePolicies
			},
		},
		{
			name: "google model armor",
			translate: func() []*api.BackendPolicySpec {
				return processGoogleModelArmor(ctx, "default", &agentgateway.GoogleModelArmor{
					Policies: &agentgateway.GoogleModelArmorPolicy{
						BackendConnectionPolicy: connection,
						Auth:                    &agentgateway.GoogleModelArmorAuth{GCP: &agentgateway.GcpAuth{}},
					},
				}).InlinePolicies
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			policies := tt.translate()
			require.Len(t, policies, 3)
			var tls *api.BackendPolicySpec_BackendTLS
			var auth *api.BackendAuthPolicy
			var tunnel *api.BackendPolicySpec_BackendTunnel
			for _, policy := range policies {
				if p := policy.GetBackendTls(); p != nil {
					tls = p
				}
				if p := policy.GetAuth(); p != nil {
					auth = p
				}
				if p := policy.GetBackendTunnel(); p != nil {
					tunnel = p
				}
			}
			require.NotNil(t, tls)
			require.Equal(t, []api.TLSConfig_KeyExchangeGroup{api.TLSConfig_X25519}, tls.KeyExchangeGroups)
			require.NotNil(t, auth)
			require.NotNil(t, tunnel)
			require.Equal(t, "proxy.example.com", tunnel.Proxy.GetInline().Hostname)
			require.Equal(t, api.BackendPolicySpec_BackendTunnel_CONNECT, tunnel.Mode)
		})
	}
}

func fipsTLSPolicyContext(t *testing.T) (*AgwCollections, ReferenceIndex, *agentgateway.AgentgatewayPolicy) {
	t.Helper()
	collections := backendTLSContext(t, nil, nil).Collections
	collections.Gateways = krt.NewStaticCollection[*gwv1.Gateway](nil, []*gwv1.Gateway{{
		Name: "gateway", Namespace: "default",
		Spec: gwv1.GatewaySpec{GatewayClassName: "agentgateway"},
	}}, krt.WithName("plugins/fipsTLSGateways"))
	collections.GatewayClasses = krt.NewStaticCollection[*gwv1.GatewayClass](nil, []*gwv1.GatewayClass{{
		Name: "agentgateway",
		Spec: gwv1.GatewayClassSpec{ControllerName: gwv1.GatewayController(collections.ControllerName)},
	}}, krt.WithName("plugins/fipsTLSGatewayClasses"))
	references := BuildReferenceIndex(nil, nil, DefaultReferenceTypes(collections))
	policy := &agentgateway.AgentgatewayPolicy{
		Name: "policy", Namespace: "default", Generation: 1,
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []agentgateway.LocalPolicyTargetReferenceWithSectionName{{
				Group: wellknown.GatewayGroup, Kind: wellknown.GatewayKind, Name: "gateway",
			}},
		},
	}
	return collections, references, policy
}
