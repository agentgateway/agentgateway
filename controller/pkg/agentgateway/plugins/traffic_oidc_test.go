package plugins

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

type staticOIDCLookup struct {
	provider *oidc.DiscoveredProvider
}

func (s staticOIDCLookup) ResolveForOwner(krt.HandlerContext, oidc.RemoteOidcOwner) (*oidc.DiscoveredProvider, error) {
	return s.provider, nil
}

func TestOIDCPolicyKeyFormatIsStable(t *testing.T) {
	// The dataplane derives the OIDC session PolicyId — hashed into browser
	// cookie names — as "policy/" + this Key. Changing the format invalidates
	// every live session on upgrade; change it together with
	// cookie_names_are_stable_for_the_cross_plane_policy_key_contract in
	// crates/agentgateway/src/http/oidc/tests.rs.
	policy, err := processOIDCPolicy(
		PolicyCtx{OidcLookup: staticOIDCLookup{provider: &oidc.DiscoveredProvider{
			IssuerURL:             "https://issuer.example",
			AuthorizationEndpoint: "https://issuer.example/auth",
			TokenEndpoint:         "https://issuer.example/token",
			JwksInline:            `{"keys":[]}`,
		}}},
		&agentgateway.OIDC{
			IssuerURL:   "https://issuer.example",
			ClientID:    "client",
			RedirectURI: "https://app.example/callback",
		},
		nil,
		types.NamespacedName{Namespace: "default", Name: "test"},
	)
	require.NoError(t, err)
	require.Equal(t, "default/test:oidc", policy.Key)
}

func TestConfiguredOIDCTokenEndpointAuth(t *testing.T) {
	tests := []struct {
		name            string
		method          agentgateway.OIDCTokenEndpointAuthMethod
		hasClientSecret bool
		want            api.TrafficPolicySpec_OIDC_TokenEndpointAuth
		wantErrContains string
	}{
		{
			name:            "ClientSecretBasic with secret",
			method:          agentgateway.OIDCTokenEndpointAuthMethodClientSecretBasic,
			hasClientSecret: true,
			want:            api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC,
		},
		{
			name:            "ClientSecretBasic without secret rejected",
			method:          agentgateway.OIDCTokenEndpointAuthMethodClientSecretBasic,
			hasClientSecret: false,
			wantErrContains: "requires a clientSecret",
		},
		{
			name:            "ClientSecretPost with secret",
			method:          agentgateway.OIDCTokenEndpointAuthMethodClientSecretPost,
			hasClientSecret: true,
			want:            api.TrafficPolicySpec_OIDC_CLIENT_SECRET_POST,
		},
		{
			name:            "ClientSecretPost without secret rejected",
			method:          agentgateway.OIDCTokenEndpointAuthMethodClientSecretPost,
			hasClientSecret: false,
			wantErrContains: "requires a clientSecret",
		},
		{
			name:            "None without secret",
			method:          agentgateway.OIDCTokenEndpointAuthMethodNone,
			hasClientSecret: false,
			want:            api.TrafficPolicySpec_OIDC_NONE,
		},
		{
			name:            "None with secret rejected",
			method:          agentgateway.OIDCTokenEndpointAuthMethodNone,
			hasClientSecret: true,
			wantErrContains: "must not be paired with a clientSecret",
		},
		{
			name:            "unsupported method rejected",
			method:          "PrivateKeyJWT",
			hasClientSecret: true,
			wantErrContains: "unsupported tokenEndpointAuthMethod",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, err := configuredOIDCTokenEndpointAuth(
				&agentgateway.OIDC{TokenEndpointAuthMethod: new(tc.method)},
				tc.hasClientSecret,
			)

			if tc.wantErrContains != "" {
				require.Error(t, err)
				require.Contains(t, err.Error(), tc.wantErrContains)
				return
			}
			require.NoError(t, err)
			require.Equal(t, tc.want, got)
		})
	}
}

func TestResolveOIDCTokenEndpointAuth(t *testing.T) {
	// Discovery-based selection now lives in the dataplane
	// (parse_token_endpoint_auth_methods); the controller only resolves an
	// explicit override and otherwise emits UNSPECIFIED + the advertised list.
	t.Run("explicit override is validated and resolved by the controller", func(t *testing.T) {
		method := agentgateway.OIDCTokenEndpointAuthMethodClientSecretBasic
		got, err := resolveOIDCTokenEndpointAuth(
			&agentgateway.OIDC{TokenEndpointAuthMethod: &method},
			true,
		)
		require.NoError(t, err)
		require.Equal(t, api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC, got)
	})

	t.Run("no override defers selection to the dataplane via UNSPECIFIED", func(t *testing.T) {
		for _, hasSecret := range []bool{true, false} {
			got, err := resolveOIDCTokenEndpointAuth(&agentgateway.OIDC{}, hasSecret)
			require.NoError(t, err)
			require.Equal(t, api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, got)
		}
	})
}

func TestResolveOIDCClientSecret(t *testing.T) {
	const (
		policyNs   = "default"
		secretName = "oidc-secret"
	)

	makeCtx := func(t *testing.T, secret *corev1.Secret) PolicyCtx {
		t.Helper()
		var secrets []*corev1.Secret
		if secret != nil {
			secrets = []*corev1.Secret{secret}
		}
		secretsCol := krt.NewStaticCollection(nil, secrets)
		return PolicyCtx{
			Krt:                krt.TestingDummyContext{},
			Collections:        &AgwCollections{Secrets: secretsCol},
			CredentialResolver: kubeutils.NewSecretCredentialResolver(secretsCol),
		}
	}

	cfg := &agentgateway.OIDC{
		ClientSecret: &corev1.LocalObjectReference{Name: secretName},
	}

	t.Run("returns secret value when present", func(t *testing.T) {
		secret := &corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{Namespace: policyNs, Name: secretName},
			Data:       map[string][]byte{"clientSecret": []byte("s3cr3t")},
		}

		got, err := resolveOIDCClientSecret(makeCtx(t, secret), policyNs, cfg)

		require.NoError(t, err)
		require.Equal(t, "s3cr3t", got)
	})

	t.Run("missing secret returns typed error", func(t *testing.T) {
		_, err := resolveOIDCClientSecret(makeCtx(t, nil), policyNs, cfg)

		require.ErrorContains(t, err, "not found")
	})

	t.Run("missing data key returns typed error", func(t *testing.T) {
		secret := &corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{Namespace: policyNs, Name: secretName},
			Data:       map[string][]byte{"other": []byte("v")},
		}

		_, err := resolveOIDCClientSecret(makeCtx(t, secret), policyNs, cfg)

		require.ErrorContains(t, err, "missing or has empty")
	})

	t.Run("empty data value returns typed error", func(t *testing.T) {
		secret := &corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{Namespace: policyNs, Name: secretName},
			Data:       map[string][]byte{"clientSecret": []byte("   ")},
		}

		_, err := resolveOIDCClientSecret(makeCtx(t, secret), policyNs, cfg)

		require.ErrorContains(t, err, "empty")
	})

	t.Run("nil clientSecret returns empty", func(t *testing.T) {
		got, err := resolveOIDCClientSecret(makeCtx(t, nil), policyNs, &agentgateway.OIDC{})

		require.NoError(t, err)
		require.Empty(t, got)
	})
}
