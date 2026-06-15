package oidc

import (
	"fmt"
	"net/url"
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

const discoveryPath = "/.well-known/openid-configuration"

// ResolvedOidcRequest is the input to the shared-request collapse step.
// Target may differ from ExpectedIssuer when BackendRef is used.
type ResolvedOidcRequest struct {
	OwnerID        remotecache.OwnerID
	Target         remotehttp.ResolvedTarget
	ExpectedIssuer string
	// ViaBackendRef is true when provider calls go through a resolved backend
	// transport (Target points at the backend) instead of the provider URLs.
	ViaBackendRef bool
	TTL           time.Duration
}

// RequestKey derives the fetch key from the resolved trust identity. A method
// rather than a field so the fetch-request collection and the
// translation-time lookup cannot diverge on key derivation.
func (r *ResolvedOidcRequest) RequestKey() remotehttp.FetchKey {
	return oidcRequestKey(r.Target.Target, r.ExpectedIssuer, r.ViaBackendRef)
}

// Resolver translates a RemoteOidcOwner into a resolved fetch request:
// discovery URL plus any per-backend TLS from an attached policy.
type Resolver interface {
	ResolveOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*ResolvedOidcRequest, error)
}

type defaultResolver struct {
	endpointResolver remotehttp.Resolver
}

// NewResolver wires OIDC to the shared remotehttp.Resolver so BackendRef OIDC
// fetches reuse the same backend-to-TLS plumbing as JWKS and MCP auth.
func NewResolver(endpointResolver remotehttp.Resolver) Resolver {
	return &defaultResolver{endpointResolver: endpointResolver}
}

func (r *defaultResolver) ResolveOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
	discURL, err := discoveryURL(owner.Config.IssuerURL)
	if err != nil {
		return nil, err
	}

	endpoint, err := resolveEndpoint(krtctx, r.endpointResolver, owner.ID.Name, owner.ID.Namespace, owner.Config, discURL)
	if err != nil {
		return nil, err
	}

	return &ResolvedOidcRequest{
		OwnerID:        owner.ID,
		Target:         *endpoint,
		ExpectedIssuer: owner.Config.IssuerURL,
		ViaBackendRef:  owner.Config.BackendRef != nil,
		TTL:            owner.TTL,
	}, nil
}

// discoveryURL constructs the OIDC well-known configuration endpoint URL from the given issuer URL.
// It verifies that the issuer URL is a valid, absolute HTTPS URL with no query or fragment.
func discoveryURL(issuerURL string) (string, error) {
	u, err := url.Parse(issuerURL)
	if err != nil {
		return "", fmt.Errorf("invalid issuer URL %q: %w", issuerURL, err)
	}
	if !u.IsAbs() || u.Scheme != "https" || u.Host == "" || u.RawQuery != "" || u.Fragment != "" {
		return "", fmt.Errorf("issuer URL must be absolute HTTPS with a host and no query or fragment")
	}
	return u.JoinPath(discoveryPath).String(), nil
}

// resolveEndpoint produces the transport target for an OIDC discovery fetch.
// Without BackendRef: hit discoveryURL with the system trust store.
// With BackendRef: resolver derives URL + TLS from the referenced Service or
// Backend, applying attached AgentgatewayPolicy backend TLS settings.
func resolveEndpoint(
	krtctx krt.HandlerContext,
	resolver remotehttp.Resolver,
	policyName, defaultNS string,
	cfg agentgateway.OIDC,
	discURL string,
) (*remotehttp.ResolvedTarget, error) {
	if cfg.BackendRef == nil {
		target := remotehttp.FetchTarget{URL: discURL}
		return &remotehttp.ResolvedTarget{Target: target}, nil
	}

	if resolver == nil {
		return nil, remotehttp.ErrResolverNotInitialized
	}

	return resolver.Resolve(krtctx, remotehttp.ResolveInput{
		ParentName:       policyName,
		DefaultNamespace: defaultNS,
		BackendRef:       *cfg.BackendRef,
		Path:             discoveryPath,
	})
}
