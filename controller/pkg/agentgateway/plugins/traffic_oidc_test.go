package plugins

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil/krttest"
)

func TestNormalizedOIDCScopesAlwaysIncludesOpenidFirst(t *testing.T) {
	tests := []struct {
		name string
		in   []string
		want []string
	}{
		{
			name: "empty input",
			in:   nil,
			want: []string{"openid"},
		},
		{
			name: "without openid",
			in:   []string{"email", "profile"},
			want: []string{"openid", "email", "profile"},
		},
		{
			name: "openid already present",
			in:   []string{"openid", "email"},
			want: []string{"openid", "email"},
		},
		{
			name: "openid present at non-first position is preserved at first",
			in:   []string{"email", "openid", "profile"},
			want: []string{"openid", "email", "profile"},
		},
		{
			name: "duplicates collapsed in input order",
			in:   []string{"email", "profile", "email", "openid"},
			want: []string{"openid", "email", "profile"},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			require.Equal(t, tc.want, normalizedOIDCScopes(tc.in))
		})
	}
}

func TestConfiguredOIDCTokenEndpointAuth(t *testing.T) {
	stringPtr := func(s string) *string { return &s }

	tests := []struct {
		name            string
		method          string
		hasClientSecret bool
		want            api.TrafficPolicySpec_OIDC_TokenEndpointAuth
		wantErrContains string
	}{
		{
			name:            "ClientSecretBasic with secret",
			method:          oidcConfigTokenEndpointAuthMethodClientSecretBasic,
			hasClientSecret: true,
			want:            api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC,
		},
		{
			name:            "ClientSecretBasic without secret rejected",
			method:          oidcConfigTokenEndpointAuthMethodClientSecretBasic,
			hasClientSecret: false,
			wantErrContains: "requires a clientSecret",
		},
		{
			name:            "ClientSecretPost with secret",
			method:          oidcConfigTokenEndpointAuthMethodClientSecretPost,
			hasClientSecret: true,
			want:            api.TrafficPolicySpec_OIDC_CLIENT_SECRET_POST,
		},
		{
			name:            "ClientSecretPost without secret rejected",
			method:          oidcConfigTokenEndpointAuthMethodClientSecretPost,
			hasClientSecret: false,
			wantErrContains: "requires a clientSecret",
		},
		{
			name:            "None without secret",
			method:          oidcConfigTokenEndpointAuthMethodNone,
			hasClientSecret: false,
			want:            api.TrafficPolicySpec_OIDC_NONE,
		},
		{
			name:            "None with secret rejected",
			method:          oidcConfigTokenEndpointAuthMethodNone,
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
				&agentgateway.OIDC{TokenEndpointAuthMethod: stringPtr(tc.method)},
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

func TestDiscoveredOIDCTokenEndpointAuth(t *testing.T) {
	tests := []struct {
		name            string
		methods         []string
		hasClientSecret bool
		want            api.TrafficPolicySpec_OIDC_TokenEndpointAuth
		wantErrContains string
	}{
		{
			name:            "advertised list empty + secret defaults to client_secret_basic",
			methods:         nil,
			hasClientSecret: true,
			want:            api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC,
		},
		{
			name:            "advertised list empty + no secret requires explicit none",
			methods:         nil,
			hasClientSecret: false,
			wantErrContains: "does not advertise",
		},
		{
			name:            "secret prefers basic over post",
			methods:         []string{"client_secret_post", "client_secret_basic"},
			hasClientSecret: true,
			want:            api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC,
		},
		{
			name:            "secret falls back to post when basic missing",
			methods:         []string{"client_secret_post", "private_key_jwt"},
			hasClientSecret: true,
			want:            api.TrafficPolicySpec_OIDC_CLIENT_SECRET_POST,
		},
		{
			name:            "secret with no supported confidential method rejected",
			methods:         []string{"private_key_jwt"},
			hasClientSecret: true,
			wantErrContains: "does not advertise",
		},
		{
			name:            "no secret picks none when advertised",
			methods:         []string{"none", "private_key_jwt"},
			hasClientSecret: false,
			want:            api.TrafficPolicySpec_OIDC_NONE,
		},
		{
			name:            "no secret rejects when none not advertised",
			methods:         []string{"client_secret_basic", "client_secret_post"},
			hasClientSecret: false,
			wantErrContains: "does not advertise",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, err := discoveredOIDCTokenEndpointAuth(tc.methods, tc.hasClientSecret)

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
		secretsCol := krt.NewStaticCollection(krttest.AlwaysSynced{}, secrets)
		return PolicyCtx{
			Krt:         krt.TestingDummyContext{},
			Collections: &AgwCollections{Secrets: secretsCol},
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
