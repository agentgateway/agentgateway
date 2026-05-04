package oidc

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type Lookup interface {
	ResolveForOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*DiscoveredProvider, error)
}

type lookupImpl struct {
	persisted *PersistedEntries
}

// NewLookup resolves providers from the KRT-backed persisted provider
// collection so translation recomputes when discovery persistence changes.
func NewLookup(persisted *PersistedEntries) Lookup {
	return &lookupImpl{persisted: persisted}
}

func (l *lookupImpl) ResolveForOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*DiscoveredProvider, error) {
	if l.persisted == nil {
		return nil, fmt.Errorf("oidc provider lookup is not configured")
	}

	discoveryURL, err := OidcDiscoveryURL(owner.Config.IssuerURL)
	if err != nil {
		return nil, err
	}
	target := remotehttp.FetchTarget{URL: discoveryURL}

	provider, ok := l.persisted.CanonicalGet(krtctx, oidcRequestKey(target, owner.Config.IssuerURL))
	if !ok {
		return nil, fmt.Errorf("oidc provider for %q isn't available (not yet fetched or fetch failed)", target.URL)
	}
	return &provider, nil
}
