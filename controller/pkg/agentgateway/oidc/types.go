// Package oidc implements a controller-side OIDC discovery + JWKS pre-fetch
// mechanism that mirrors the controller/pkg/agentgateway/jwks package.
// The controller performs OpenID Connect discovery and JWKS pre-fetch so the
// dataplane never calls .well-known/openid-configuration or jwks_uri directly.
package oidc

import (
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"reflect"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// DiscoveredProvider holds the result of a successful OIDC discovery fetch.
// It includes both the discovery document metadata and the pre-fetched JWKS
// so the dataplane never needs to call either endpoint.
type DiscoveredProvider struct {
	// RequestKey is the canonical key identifying this provider in the store.
	RequestKey remotehttp.FetchKey `json:"requestKey"`
	// IssuerURL is the discovered issuer URL (validated against user-configured).
	IssuerURL string `json:"issuerURL"`
	// AuthorizationEndpoint is the IdP authorization endpoint.
	AuthorizationEndpoint string `json:"authorizationEndpoint"`
	// TokenEndpoint is the IdP token endpoint.
	TokenEndpoint string `json:"tokenEndpoint"`
	// JwksURI is the IdP JWKS endpoint (fetched and inlined as JwksJSON).
	JwksURI string `json:"jwksURI"`
	// JwksJSON is the pre-fetched JWKS JSON blob.
	JwksJSON string `json:"jwksJSON"`
	// TokenEndpointAuthMethodsSupported lists the auth methods the IdP advertises.
	TokenEndpointAuthMethodsSupported []string `json:"tokenEndpointAuthMethodsSupported,omitempty"`
	// FetchedAt is when the discovery was last successfully fetched.
	FetchedAt time.Time `json:"fetchedAt"`
}

// OwnerKey identifies the owner of an OIDC discovery request.
type OwnerKey = OidcOwnerID

// OidcSource is a per-owner OIDC discovery request before KRT collapses
// equivalent sources onto a shared request key.
type OidcSource struct {
	OwnerKey       OwnerKey
	RequestKey     remotehttp.FetchKey
	ExpectedIssuer string
	Target         remotehttp.FetchTarget
	// +noKrtEquals
	TLSConfig *tls.Config
	// +noKrtEquals
	ProxyTLSConfig *tls.Config
	TTL            time.Duration
}

func (s OidcSource) ResourceName() string {
	return s.OwnerKey.String()
}

func (s OidcSource) Equals(other OidcSource) bool {
	return s.OwnerKey == other.OwnerKey &&
		s.RequestKey == other.RequestKey &&
		s.ExpectedIssuer == other.ExpectedIssuer &&
		reflect.DeepEqual(s.Target, other.Target) &&
		s.TTL == other.TTL
}

// SharedOidcRequest is the canonical OIDC discovery request produced by KRT
// for a shared fetch key. It is the unit the runtime Fetcher and persistence
// layer watch.
type SharedOidcRequest struct {
	RequestKey     remotehttp.FetchKey
	ExpectedIssuer string
	Target         remotehttp.FetchTarget
	// +noKrtEquals
	TLSConfig *tls.Config
	// +noKrtEquals
	ProxyTLSConfig *tls.Config
	TTL            time.Duration
}

func (r SharedOidcRequest) ResourceName() string {
	return string(r.RequestKey)
}

func (r SharedOidcRequest) Equals(other SharedOidcRequest) bool {
	return r.RequestKey == other.RequestKey &&
		r.ExpectedIssuer == other.ExpectedIssuer &&
		reflect.DeepEqual(r.Target, other.Target) &&
		r.TTL == other.TTL
}

// OidcSource returns the canonical runtime request consumed by the Fetcher.
func (r SharedOidcRequest) OidcSource() OidcSource {
	return OidcSource{
		RequestKey:     r.RequestKey,
		ExpectedIssuer: r.ExpectedIssuer,
		Target:         r.Target,
		TLSConfig:      r.TLSConfig,
		ProxyTLSConfig: r.ProxyTLSConfig,
		TTL:            r.TTL,
	}
}

func oidcRequestKey(target remotehttp.FetchTarget, expectedIssuer string) remotehttp.FetchKey {
	hash := sha256.New()
	writeHashPart := func(value string) {
		_, _ = hash.Write([]byte(value))
		_, _ = hash.Write([]byte{0})
	}

	writeHashPart(target.Key().String())
	writeHashPart(expectedIssuer)

	return remotehttp.FetchKey(hex.EncodeToString(hash.Sum(nil)))
}
