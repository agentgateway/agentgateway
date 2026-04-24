package oidc

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
)

func TestOwnersFromPolicyRequiresTargetRefs(t *testing.T) {
	policy := &agentgateway.AgentgatewayPolicy{}
	policy.Namespace = "default"
	policy.Name = "example"
	// No TargetRefs
	policy.Spec.Traffic = &agentgateway.Traffic{
		OIDC: &agentgateway.OIDC{
			IssuerURL:   "https://issuer.example",
			ClientID:    "my-client",
			RedirectURI: "https://app.example/callback",
		},
	}

	assert.Nil(t, OwnersFromPolicy(policy))
}

func TestOwnersFromPolicyRequiresOIDCField(t *testing.T) {
	policy := &agentgateway.AgentgatewayPolicy{}
	policy.Namespace = "default"
	policy.Name = "example"
	policy.Spec.TargetRefs = make([]shared.LocalPolicyTargetReferenceWithSectionName, 1)
	// No Traffic.OIDC

	assert.Nil(t, OwnersFromPolicy(policy))
}

func TestOwnersFromPolicyExtractsOIDCOwner(t *testing.T) {
	policy := &agentgateway.AgentgatewayPolicy{}
	policy.Namespace = "default"
	policy.Name = "example"
	policy.Spec.TargetRefs = make([]shared.LocalPolicyTargetReferenceWithSectionName, 1)

	backend := gwv1.BackendObjectReference{
		Name: "my-backend",
	}
	secretRef := corev1.LocalObjectReference{Name: "my-secret"}
	scopes := []string{"openid", "profile"}
	policy.Spec.Traffic = &agentgateway.Traffic{
		OIDC: &agentgateway.OIDC{
			IssuerURL:    "https://issuer.example",
			ClientID:     "my-client",
			ClientSecret: &secretRef,
			RedirectURI:  "https://app.example/callback",
			Scopes:       scopes,
			Backend:      &backend,
			RefreshInterval: &metav1.Duration{
				Duration: 30 * time.Minute,
			},
		},
	}

	owners := OwnersFromPolicy(policy)
	assert.Len(t, owners, 1)
	assert.Equal(t, "AgentgatewayPolicy/default/example#spec.traffic.oidc", owners[0].ID.String())
	assert.Equal(t, "default", owners[0].DefaultNamespace)
	assert.Equal(t, 30*time.Minute, owners[0].TTL)
	assert.Equal(t, "https://issuer.example", owners[0].Config.IssuerURL)
}

func TestOidcOwnerIDString(t *testing.T) {
	id := OidcOwnerID{
		Namespace: "ns",
		Name:      "pol",
		Path:      "spec.traffic.oidc",
	}
	assert.Equal(t, "AgentgatewayPolicy/ns/pol#spec.traffic.oidc", id.String())
}

func TestTTLForOIDCDefaultsToOneHour(t *testing.T) {
	cfg := agentgateway.OIDC{}
	assert.Equal(t, time.Hour, TTLForOIDC(cfg))
}

func TestTTLForOIDCUsesConfiguredRefreshInterval(t *testing.T) {
	cfg := agentgateway.OIDC{
		RefreshInterval: &metav1.Duration{Duration: 15 * time.Minute},
	}
	assert.Equal(t, 15*time.Minute, TTLForOIDC(cfg))
}

func TestRemoteOidcOwnerEquality(t *testing.T) {
	secret := corev1.LocalObjectReference{Name: "secret"}
	backendName := gwv1.ObjectName("oidc-backend")
	backendKind := gwv1.Kind("Service")
	refreshInterval := &metav1.Duration{Duration: 15 * time.Minute}
	tokenAuthMethod := "ClientSecretPost"

	base := RemoteOidcOwner{
		ID:               OidcOwnerID{Namespace: "ns", Name: "pol", Path: "spec.traffic.oidc"},
		DefaultNamespace: "ns",
		Config: agentgateway.OIDC{
			IssuerURL:               "https://issuer.example",
			ClientID:                "c1",
			ClientSecret:            &secret,
			RedirectURI:             "https://app/callback",
			Scopes:                  []string{"openid", "profile"},
			Backend:                 &gwv1.BackendObjectReference{Name: backendName, Kind: &backendKind},
			RefreshInterval:         refreshInterval,
			TokenEndpointAuthMethod: &tokenAuthMethod,
		},
		TTL: time.Hour,
	}

	t.Run("equal to itself", func(t *testing.T) {
		assert.True(t, base.Equals(base))
	})

	t.Run("different TTL not equal", func(t *testing.T) {
		other := base
		other.TTL = 30 * time.Minute
		assert.False(t, base.Equals(other))
	})

	t.Run("different issuer not equal", func(t *testing.T) {
		other := base
		other.Config.IssuerURL = "https://other.example"
		assert.False(t, base.Equals(other))
	})

	t.Run("different backend not equal", func(t *testing.T) {
		other := base
		other.Config.Backend = &gwv1.BackendObjectReference{Name: "other-backend", Kind: &backendKind}
		assert.False(t, base.Equals(other))
	})

	t.Run("different token auth method not equal", func(t *testing.T) {
		other := base
		method := "ClientSecretBasic"
		other.Config.TokenEndpointAuthMethod = &method
		assert.False(t, base.Equals(other))
	})

	t.Run("nil and empty scopes compare equal", func(t *testing.T) {
		withoutScopes := base
		withoutScopes.Config.Scopes = nil

		withEmptyScopes := base
		withEmptyScopes.Config.Scopes = []string{}

		assert.True(t, withoutScopes.Equals(withEmptyScopes))
	})
}
