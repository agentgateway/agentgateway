// Package oidc derives controller-side OIDC discovery requests from
// AgentgatewayPolicy and adapts resolved provider metadata into xDS.
package oidc

import (
	"crypto/tls"
	"encoding/json"
	"time"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

const (
	oidcRequestKeyDomain = "oidc-discovery"
)

// DiscoveredProvider holds the metadata from a successful OIDC discovery fetch,
// including the JWKS key material fetched from the discovery document's jwks_uri.
type DiscoveredProvider struct {
	RequestKey remotehttp.FetchKey `json:"requestKey"`
	// IssuerURL is the IdP-reported issuer, validated byte-exact against the user-configured issuer.
	IssuerURL                         string    `json:"issuerURL"`
	AuthorizationEndpoint             string    `json:"authorizationEndpoint"`
	TokenEndpoint                     string    `json:"tokenEndpoint"`
	JwksURI                           string    `json:"jwksURI"`
	JwksInline                        string    `json:"jwksInline"`
	TokenEndpointAuthMethodsSupported []string  `json:"tokenEndpointAuthMethodsSupported,omitempty"`
	FetchedAt                         time.Time `json:"fetchedAt"`
}

func (p DiscoveredProvider) RemoteRequestKey() remotehttp.FetchKey {
	return p.RequestKey
}

func (p DiscoveredProvider) RemoteFetchedAt() time.Time {
	return p.FetchedAt
}

func (p DiscoveredProvider) ResourceName() string { return string(p.RequestKey) }

// Equals compares JwksInline last so the multi-KB string compare short-circuits
// on cheaper-field diffs.
func (p DiscoveredProvider) Equals(other DiscoveredProvider) bool {
	return p.RequestKey == other.RequestKey &&
		p.IssuerURL == other.IssuerURL &&
		p.AuthorizationEndpoint == other.AuthorizationEndpoint &&
		p.TokenEndpoint == other.TokenEndpoint &&
		p.JwksURI == other.JwksURI &&
		slices.Equal(p.TokenEndpointAuthMethodsSupported, other.TokenEndpointAuthMethodsSupported) &&
		p.FetchedAt.Equal(other.FetchedAt) &&
		p.JwksInline == other.JwksInline
}

var (
	_ krt.ResourceNamer               = DiscoveredProvider{}
	_ krt.Equaler[DiscoveredProvider] = DiscoveredProvider{}
)

// oidcRequestSpec is the shared identity of an OIDC discovery request: the
// fields that decide what to fetch and how to reach it. OidcSource adds the
// per-owner key; SharedOidcRequest is the collapsed canonical form. Both embed
// this so a field added here automatically participates in equality, the krt
// snapshot JSON, and the runtime request for both — no lockstep duplication.
type oidcRequestSpec struct {
	RequestKey     remotehttp.FetchKey
	ExpectedIssuer string
	Target         remotehttp.FetchTarget
	// ViaBackendRef is true when provider calls go through a resolved backend
	// transport (Target points at the backend) instead of the provider URLs.
	ViaBackendRef bool
	// +noKrtEquals
	TLSConfig *tls.Config
	// +noKrtEquals
	ProxyTLSConfig *tls.Config
	TTL            time.Duration
}

func (s oidcRequestSpec) RemoteRequestKey() remotehttp.FetchKey { return s.RequestKey }
func (s oidcRequestSpec) RemoteTTL() time.Duration              { return s.TTL }

func (s oidcRequestSpec) equals(other oidcRequestSpec) bool {
	return s.RequestKey == other.RequestKey &&
		s.ExpectedIssuer == other.ExpectedIssuer &&
		s.Target.Equals(other.Target) &&
		s.ViaBackendRef == other.ViaBackendRef &&
		s.TTL == other.TTL
}

// oidcRequestJSON is the krt-snapshot view of a spec: the embedded *tls.Config
// objects are not serializable, so their presence is reported as booleans.
type oidcRequestJSON struct {
	RequestKey        remotehttp.FetchKey    `json:"requestKey"`
	ExpectedIssuer    string                 `json:"expectedIssuer"`
	Target            remotehttp.FetchTarget `json:"target"`
	ViaBackendRef     bool                   `json:"viaBackendRef"`
	HasTLSConfig      bool                   `json:"hasTLSConfig"`
	HasProxyTLSConfig bool                   `json:"hasProxyTLSConfig"`
	TTL               time.Duration          `json:"ttl"`
}

func (s oidcRequestSpec) snapshot() oidcRequestJSON {
	return oidcRequestJSON{
		RequestKey:        s.RequestKey,
		ExpectedIssuer:    s.ExpectedIssuer,
		Target:            s.Target,
		ViaBackendRef:     s.ViaBackendRef,
		HasTLSConfig:      s.TLSConfig != nil,
		HasProxyTLSConfig: s.ProxyTLSConfig != nil,
		TTL:               s.TTL,
	}
}

// OidcSource is a per-owner OIDC discovery request before KRT collapses
// equivalent sources onto a shared request key.
type OidcSource struct {
	OwnerKey remotecache.OwnerID
	oidcRequestSpec
}

func (s OidcSource) ResourceName() string { return s.OwnerKey.String() }

func (s OidcSource) Equals(other OidcSource) bool {
	return s.OwnerKey == other.OwnerKey && s.oidcRequestSpec.equals(other.oidcRequestSpec)
}

func (s OidcSource) MarshalJSON() ([]byte, error) {
	return json.Marshal(struct {
		OwnerKey remotecache.OwnerID `json:"ownerKey"`
		oidcRequestJSON
	}{OwnerKey: s.OwnerKey, oidcRequestJSON: s.oidcRequestSpec.snapshot()})
}

// SharedOidcRequest is the canonical OIDC discovery request KRT produces per
// shared fetch key; the unit the Fetcher and persistence layer watch.
type SharedOidcRequest struct {
	oidcRequestSpec
}

func (r SharedOidcRequest) ResourceName() string { return string(r.RequestKey) }

func (r SharedOidcRequest) Equals(other SharedOidcRequest) bool {
	return r.oidcRequestSpec.equals(other.oidcRequestSpec)
}

func (r SharedOidcRequest) MarshalJSON() ([]byte, error) {
	return json.Marshal(r.oidcRequestSpec.snapshot())
}

// oidcRequestKey domain-separates by (target, expectedIssuer, transport
// mode) — all three are part of trust identity.
func oidcRequestKey(target remotehttp.FetchTarget, expectedIssuer string, viaBackendRef bool) remotehttp.FetchKey {
	parts := []string{oidcRequestKeyDomain, target.Key().String(), expectedIssuer}
	if viaBackendRef {
		parts = append(parts, "provider-backend")
	} else {
		parts = append(parts, "direct")
	}
	return remotehttp.HashKey(parts...)
}
