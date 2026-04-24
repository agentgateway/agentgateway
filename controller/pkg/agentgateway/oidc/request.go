package oidc

import (
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// ResolvedOidcRequest is the per-owner resolved discovery request, carrying the
// resolved target URL and TLS configuration for the OIDC discovery endpoint.
type ResolvedOidcRequest struct {
	OwnerID OidcOwnerID
	// ExpectedIssuer is the user-configured `issuerURL` preserved byte-for-byte
	// for the OIDC Discovery §3 / §4.3 issuer-match check; do not normalize.
	ExpectedIssuer string
	Target         remotehttp.ResolvedTarget
	TTL            time.Duration
}

// Resolver resolves a RemoteOidcOwner to a ResolvedOidcRequest.
type Resolver interface {
	ResolveOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*ResolvedOidcRequest, error)
}
