package oidc

import (
	"errors"
	"fmt"
	"net/url"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

var (
	errResolverNotInitialized = errors.New("remote http resolver hasn't been initialized")
)

// defaultResolver implements Resolver using the shared remotehttp.Resolver.
type defaultResolver struct {
	endpointResolver remotehttp.Resolver
}

// NewResolver constructs a Resolver backed by the given remotehttp.Resolver.
func NewResolver(endpointResolver remotehttp.Resolver) Resolver {
	return &defaultResolver{endpointResolver: endpointResolver}
}

func (r *defaultResolver) ResolveOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
	endpoint, err := resolveOidcEndpoint(krtctx, r.endpointResolver, owner)
	if err != nil {
		return nil, err
	}

	return &ResolvedOidcRequest{
		OwnerID:        owner.ID,
		ExpectedIssuer: owner.Config.IssuerURL,
		Target:         *endpoint,
		TTL:            owner.TTL,
	}, nil
}

// resolveOidcEndpoint resolves the OIDC discovery URL for the given owner.
// If the owner has a Backend reference, use it; otherwise build a direct URL
// from the IssuerURL.
func resolveOidcEndpoint(
	krtctx krt.HandlerContext,
	resolver remotehttp.Resolver,
	owner RemoteOidcOwner,
) (*remotehttp.ResolvedTarget, error) {
	if resolver == nil {
		return nil, errResolverNotInitialized
	}

	issuerURL := owner.Config.IssuerURL
	discoveryPath, err := oidcDiscoveryPath(issuerURL)
	if err != nil {
		return nil, err
	}

	if owner.Config.Backend != nil {
		// Route discovery through the specified backend.
		return resolver.Resolve(krtctx, remotehttp.ResolveInput{
			ParentName:       owner.ID.Name,
			DefaultNamespace: owner.DefaultNamespace,
			BackendRef:       backendRef(owner.Config.Backend),
			Path:             discoveryPath,
		})
	}

	// Direct fetch: build the discovery URL from the issuer.
	discoveryURL, err := oidcDiscoveryURL(issuerURL)
	if err != nil {
		return nil, err
	}

	target := remotehttp.FetchTarget{URL: discoveryURL}
	return &remotehttp.ResolvedTarget{
		Key:    target.Key(),
		Target: target,
	}, nil
}

// oidcDiscoveryURL returns <issuerURL>/.well-known/openid-configuration.
func oidcDiscoveryURL(issuerURL string) (string, error) {
	u, err := url.Parse(issuerURL)
	if err != nil {
		return "", fmt.Errorf("invalid issuer URL %q: %w", issuerURL, err)
	}
	base := *u
	discoveryPath, err := oidcDiscoveryPath(issuerURL)
	if err != nil {
		return "", err
	}
	base.Path = "/" + discoveryPath
	return base.String(), nil
}

// oidcDiscoveryPath returns the OIDC discovery path for an issuer URL.
func oidcDiscoveryPath(issuerURL string) (string, error) {
	u, err := url.Parse(issuerURL)
	if err != nil {
		return "", fmt.Errorf("invalid issuer URL %q: %w", issuerURL, err)
	}
	return strings.TrimPrefix(strings.TrimRight(u.Path, "/")+"/.well-known/openid-configuration", "/"), nil
}

// requestKeyForDirectIssuer derives the canonical fetch key for an issuer's
// direct (no-backend) discovery URL. Single-sources the `(discovery URL,
// expected issuer)` pair that both the resolver and the persistence layer
// hash, so they cannot drift. The expected issuer is the user-configured
// `issuerURL` verbatim, per OIDC Discovery §3 byte-for-byte matching.
func requestKeyForDirectIssuer(issuerURL string) (remotehttp.FetchKey, error) {
	discoveryURL, err := oidcDiscoveryURL(issuerURL)
	if err != nil {
		return "", err
	}
	target := remotehttp.FetchTarget{URL: discoveryURL}
	return oidcRequestKey(target, issuerURL), nil
}

// backendRef converts a gateway BackendObjectReference to the remotehttp type.
func backendRef(b *gwv1.BackendObjectReference) gwv1.BackendObjectReference {
	if b == nil {
		return gwv1.BackendObjectReference{}
	}
	return *b
}
