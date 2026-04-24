package oidc

import (
	"cmp"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// FetchKeyIndexCollectionFunc is the KRT index option for grouping by FetchKey.
var FetchKeyIndexCollectionFunc = krt.WithIndexCollectionFromString(func(s string) remotehttp.FetchKey {
	return remotehttp.FetchKey(s)
})

// CollectionInputs holds the KRT collections and resolver needed to derive OIDC
// owners and shared requests.
type CollectionInputs struct {
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	Backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	Resolver             Resolver
	KrtOpts              krtutil.KrtOptions
}

// Collections holds the KRT-derived collections for the OIDC store.
type Collections struct {
	PolicyOwners   krt.Collection[RemoteOidcOwner]
	Owners         krt.Collection[RemoteOidcOwner]
	Sources        krt.Collection[OidcSource]
	SharedRequests krt.Collection[SharedOidcRequest]
}

// NewCollections derives all OIDC KRT collections from the given inputs.
func NewCollections(inputs CollectionInputs) Collections {
	policyOwners := krt.NewManyCollection(inputs.AgentgatewayPolicies, func(kctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) []RemoteOidcOwner {
		return OwnersFromPolicy(policy)
	}, inputs.KrtOpts.ToOptions("PolicyOidcOwners")...)

	owners := policyOwners

	sources := krt.NewCollection(owners, func(kctx krt.HandlerContext, owner RemoteOidcOwner) *OidcSource {
		resolved, err := inputs.Resolver.ResolveOwner(kctx, owner)
		if err != nil {
			logger.Error("error generating remote oidc url or tls options", "error", err, "owner", owner.ID.String())
			return nil
		}

		return &OidcSource{
			OwnerKey:       resolved.OwnerID,
			RequestKey:     oidcRequestKey(resolved.Target.Target, resolved.ExpectedIssuer),
			ExpectedIssuer: resolved.ExpectedIssuer,
			Target:         resolved.Target.Target,
			TLSConfig:      resolved.Target.TLSConfig,
			ProxyTLSConfig: resolved.Target.ProxyTLSConfig,
			TTL:            resolved.TTL,
		}
	}, inputs.KrtOpts.ToOptions("OidcSources")...)

	sourcesByRequestKey := krt.NewIndex(sources, "oidc-request-key", func(source OidcSource) []remotehttp.FetchKey {
		return []remotehttp.FetchKey{source.RequestKey}
	})
	requestGroups := sourcesByRequestKey.AsCollection(append(inputs.KrtOpts.ToOptions("OidcRequestGroups"), FetchKeyIndexCollectionFunc)...)
	sharedRequests := krt.NewCollection(requestGroups, func(kctx krt.HandlerContext, grouped krt.IndexObject[remotehttp.FetchKey, OidcSource]) *SharedOidcRequest {
		return CollapseOidcSources(grouped)
	}, inputs.KrtOpts.ToOptions("OidcRequests")...)

	return Collections{
		PolicyOwners:   policyOwners,
		Owners:         owners,
		Sources:        sources,
		SharedRequests: sharedRequests,
	}
}

// CollapseOidcSources merges a group of OidcSource values sharing a request key
// into a single SharedOidcRequest. Uses the minimum TTL across owners so the
// most conservative refresh schedule wins.
func CollapseOidcSources(grouped krt.IndexObject[remotehttp.FetchKey, OidcSource]) *SharedOidcRequest {
	if len(grouped.Objects) == 0 {
		return nil
	}

	sources := append([]OidcSource(nil), grouped.Objects...)
	sources = slices.SortFunc(sources, func(a, b OidcSource) int {
		return cmp.Compare(a.OwnerKey.String(), b.OwnerKey.String())
	})

	shared := SharedOidcRequest{
		RequestKey:     grouped.Key,
		ExpectedIssuer: sources[0].ExpectedIssuer,
		Target:         sources[0].Target,
		TLSConfig:      sources[0].TLSConfig,
		ProxyTLSConfig: sources[0].ProxyTLSConfig,
		TTL:            sources[0].TTL,
	}
	for _, source := range sources[1:] {
		if source.TTL < shared.TTL {
			shared.TTL = source.TTL
		}
	}

	return &shared
}
