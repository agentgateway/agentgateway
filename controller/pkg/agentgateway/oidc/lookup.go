package oidc

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
)

type Lookup interface {
	ResolveForOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*DiscoveredProvider, error)
}

type lookupImpl struct {
	resolver Resolver
	cache    *providerCache
}

func NewLookup(persisted *PersistedEntries, resolver Resolver) Lookup {
	return &lookupImpl{
		resolver: resolver,
		cache:    newProviderCache(persisted),
	}
}

func (l *lookupImpl) ResolveForOwner(krtctx krt.HandlerContext, owner RemoteOidcOwner) (*DiscoveredProvider, error) {
	if l.cache == nil {
		return nil, fmt.Errorf("oidc persisted cache is not configured")
	}

	resolved, err := l.resolver.ResolveOwner(krtctx, owner)
	if err != nil {
		return nil, err
	}

	// Look up by the canonical RequestKey (target + expected issuer hash) — the
	// same key the fetcher and cache use. Looking up by `resolved.Target.Key`
	// (URL-only hash) silently misses the cache entry.
	provider, ok := l.cache.Get(krtctx, oidcRequestKey(resolved.Target.Target, resolved.ExpectedIssuer))
	if !ok {
		return nil, fmt.Errorf("oidc provider for %q isn't available (not yet fetched or fetch failed)", resolved.Target.Target.URL)
	}
	return &provider, nil
}
