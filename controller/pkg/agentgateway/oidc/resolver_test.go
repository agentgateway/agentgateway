package oidc

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestResolveOidcEndpointUsesBackendResolverWhenConfigured(t *testing.T) {
	backendName := gwv1.ObjectName("oidc-backend")

	target, err := resolveOidcEndpoint(nil, remotehttpResolverFunc(func(input remotehttp.ResolveInput) (*remotehttp.ResolvedTarget, error) {
		assert.Equal(t, "policy-a", input.ParentName)
		assert.Equal(t, "default", input.DefaultNamespace)
		assert.Equal(t, gwv1.BackendObjectReference{Name: backendName}, input.BackendRef)
		assert.Equal(t, ".well-known/openid-configuration", input.Path)

		return &remotehttp.ResolvedTarget{
			Key: remotehttp.FetchTarget{URL: "https://resolved.example/.well-known/openid-configuration"}.Key(),
			Target: remotehttp.FetchTarget{
				URL: "https://resolved.example/.well-known/openid-configuration",
			},
		}, nil
	}), RemoteOidcOwner{
		ID:               OidcOwnerID{Namespace: "default", Name: "policy-a", Path: "spec.traffic.oidc"},
		DefaultNamespace: "default",
		Config: agentgateway.OIDC{
			IssuerURL: "https://issuer.example",
			Backend:   &gwv1.BackendObjectReference{Name: backendName},
		},
	})

	assert.NoError(t, err)
	assert.Equal(t, "https://resolved.example/.well-known/openid-configuration", target.Target.URL)
}

func TestResolveOidcEndpointUsesIssuerPathWithBackendResolver(t *testing.T) {
	backendName := gwv1.ObjectName("oidc-backend")

	_, err := resolveOidcEndpoint(nil, remotehttpResolverFunc(func(input remotehttp.ResolveInput) (*remotehttp.ResolvedTarget, error) {
		assert.Equal(t, "tenant-a/.well-known/openid-configuration", input.Path)
		return &remotehttp.ResolvedTarget{}, nil
	}), RemoteOidcOwner{
		ID:               OidcOwnerID{Namespace: "default", Name: "policy-a", Path: "spec.traffic.oidc"},
		DefaultNamespace: "default",
		Config: agentgateway.OIDC{
			IssuerURL: "https://issuer.example/tenant-a",
			Backend:   &gwv1.BackendObjectReference{Name: backendName},
		},
	})

	assert.NoError(t, err)
}

func TestResolveOidcEndpointBuildsDirectURLWithoutBackend(t *testing.T) {
	target, err := resolveOidcEndpoint(nil, remotehttpResolverFunc(func(input remotehttp.ResolveInput) (*remotehttp.ResolvedTarget, error) {
		t.Fatalf("unexpected backend resolver call: %#v", input)
		return nil, nil
	}), RemoteOidcOwner{
		ID:               OidcOwnerID{Namespace: "default", Name: "policy-a", Path: "spec.traffic.oidc"},
		DefaultNamespace: "default",
		Config: agentgateway.OIDC{
			IssuerURL: "https://issuer.example/tenant-a",
		},
	})

	assert.NoError(t, err)
	assert.Equal(t, "https://issuer.example/tenant-a/.well-known/openid-configuration", target.Target.URL)
}

// OIDC Discovery §3 / §4.3 mandate byte-for-byte equality between the
// configured issuer and the discovery document's `issuer` claim. A trailing
// slash must round-trip through ResolveOwner unchanged for IdPs that issue
// trailing-slash issuers.
func TestResolveOwnerPreservesTrailingSlashInExpectedIssuer(t *testing.T) {
	resolver := NewResolver(remotehttpResolverFunc(func(input remotehttp.ResolveInput) (*remotehttp.ResolvedTarget, error) {
		t.Fatalf("unexpected backend resolver call: %#v", input)
		return nil, nil
	}))

	resolved, err := resolver.ResolveOwner(nil, RemoteOidcOwner{
		ID:               OidcOwnerID{Namespace: "default", Name: "policy-a", Path: "spec.traffic.oidc"},
		DefaultNamespace: "default",
		Config:           agentgateway.OIDC{IssuerURL: "https://issuer.example/"},
	})

	assert.NoError(t, err)
	assert.Equal(t, "https://issuer.example/", resolved.ExpectedIssuer)
}

type remotehttpResolverFunc func(input remotehttp.ResolveInput) (*remotehttp.ResolvedTarget, error)

func (f remotehttpResolverFunc) Resolve(_ krt.HandlerContext, input remotehttp.ResolveInput) (*remotehttp.ResolvedTarget, error) {
	return f(input)
}
