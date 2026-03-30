package jwks

import (
	"cmp"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

var fetchKeyIndexCollectionFunc = krt.WithIndexCollectionFromString(func(s string) remotehttp.FetchKey {
	return remotehttp.FetchKey(s)
})

type CollectionInputs struct {
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	Backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	Resolver             Resolver
	KrtOpts              krtutil.KrtOptions
}

type Collections struct {
	PolicyOwners   krt.Collection[RemoteJwksOwner]
	BackendOwners  krt.Collection[RemoteJwksOwner]
	Owners         krt.Collection[RemoteJwksOwner]
	Sources        krt.Collection[JwksSource]
	SharedRequests krt.Collection[SharedJwksRequest]
}

func NewCollections(inputs CollectionInputs) Collections {
	policyOwners := krt.NewManyCollection(inputs.AgentgatewayPolicies, func(kctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) []RemoteJwksOwner {
		return OwnersFromPolicy(policy)
	}, inputs.KrtOpts.ToOptions("PolicyJwksOwners")...)
	backendOwners := krt.NewManyCollection(inputs.Backends, func(kctx krt.HandlerContext, backend *agentgateway.AgentgatewayBackend) []RemoteJwksOwner {
		return OwnersFromBackend(backend)
	}, inputs.KrtOpts.ToOptions("BackendJwksOwners")...)
	owners := krt.JoinCollection([]krt.Collection[RemoteJwksOwner]{policyOwners, backendOwners}, inputs.KrtOpts.ToOptions("JwksOwners")...)

	sources := krt.NewCollection(owners, func(kctx krt.HandlerContext, owner RemoteJwksOwner) *JwksSource {
		resolved, err := inputs.Resolver.ResolveOwner(kctx, owner)
		if err != nil {
			logger.Error("error generating remote jwks url or tls options", "error", err, "owner", owner.ID.String())
			return nil
		}

		return &JwksSource{
			OwnerKey:   resolved.OwnerID,
			RequestKey: resolved.Target.Key,
			Target:     resolved.Target.Target,
			TLSConfig:  resolved.Target.TLSConfig,
			TTL:        resolved.TTL,
		}
	}, inputs.KrtOpts.ToOptions("JwksSources")...)

	sourcesByRequestKey := krt.NewIndex(sources, "jwks-request-key", func(source JwksSource) []remotehttp.FetchKey {
		return []remotehttp.FetchKey{source.RequestKey}
	})
	requestGroups := sourcesByRequestKey.AsCollection(append(inputs.KrtOpts.ToOptions("JwksRequestGroups"), fetchKeyIndexCollectionFunc)...)
	sharedRequests := krt.NewCollection(requestGroups, func(kctx krt.HandlerContext, grouped krt.IndexObject[remotehttp.FetchKey, JwksSource]) *SharedJwksRequest {
		return collapseJwksSources(grouped)
	}, inputs.KrtOpts.ToOptions("JwksRequests")...)

	return Collections{
		PolicyOwners:   policyOwners,
		BackendOwners:  backendOwners,
		Owners:         owners,
		Sources:        sources,
		SharedRequests: sharedRequests,
	}
}

func collapseJwksSources(grouped krt.IndexObject[remotehttp.FetchKey, JwksSource]) *SharedJwksRequest {
	if len(grouped.Objects) == 0 {
		return nil
	}

	sources := append([]JwksSource(nil), grouped.Objects...)
	sources = slices.SortFunc(sources, func(a, b JwksSource) int {
		return cmp.Compare(a.OwnerKey.String(), b.OwnerKey.String())
	})

	shared := SharedJwksRequest{
		RequestKey: grouped.Key,
		Target:     sources[0].Target,
		TLSConfig:  sources[0].TLSConfig,
		TTL:        sources[0].TTL,
	}
	for _, source := range sources[1:] {
		if source.TTL < shared.TTL {
			shared.TTL = source.TTL
		}
	}

	return &shared
}
