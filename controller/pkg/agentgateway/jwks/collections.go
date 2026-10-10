package jwks

import (
	"cmp"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

var FetchKeyIndexCollectionFunc = krt.WithIndexCollectionFromString(func(s string) remotehttp.FetchKey {
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
	ResolvedOwners krt.Collection[ResolvedOwner]
	SharedRequests krt.Collection[SharedJwksRequest]
}

// ResolvedOwner retains the input and any resolution error for translation.
type ResolvedOwner struct {
	Owner  RemoteJwksOwner
	Source JwksSource
	Error  string
}

func (r ResolvedOwner) ResourceName() string {
	return r.Owner.ResourceName()
}

func (r ResolvedOwner) Equals(other ResolvedOwner) bool {
	return r.Owner.Equals(other.Owner) && r.Source.Equals(other.Source) && r.Error == other.Error
}

func NewCollections(inputs CollectionInputs) Collections {
	policyOwners := krt.NewManyCollection(inputs.AgentgatewayPolicies, func(kctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) []RemoteJwksOwner {
		return OwnersFromPolicy(policy)
	}, inputs.KrtOpts.ToOptions("jwks/PolicyOwners")...)
	backendOwners := krt.NewManyCollection(inputs.Backends, func(kctx krt.HandlerContext, backend *agentgateway.AgentgatewayBackend) []RemoteJwksOwner {
		return OwnersFromBackend(backend)
	}, inputs.KrtOpts.ToOptions("jwks/BackendOwners")...)
	owners := krt.JoinCollection([]krt.Collection[RemoteJwksOwner]{policyOwners, backendOwners}, inputs.KrtOpts.ToOptions("jwks/Owners")...)

	resolvedOwners := krt.NewCollection(owners, func(kctx krt.HandlerContext, owner RemoteJwksOwner) *ResolvedOwner {
		result := &ResolvedOwner{Owner: owner}
		resolved, err := inputs.Resolver.ResolveOwner(kctx, owner)
		if err != nil {
			logger.Error("error generating remote jwks url or tls options", "error", err, "owner", owner.ID.String())
			result.Error = err.Error()
			return result
		}

		result.Source = JwksSource{
			OwnerKey:       resolved.OwnerID,
			RequestKey:     resolved.Target.Key,
			Target:         resolved.Target.Target,
			TLSConfig:      resolved.Target.TLSConfig,
			ProxyTLSConfig: resolved.Target.ProxyTLSConfig,
			TTL:            resolved.TTL,
		}
		return result
	}, inputs.KrtOpts.ToOptions("jwks/ResolvedOwners")...)
	ownersByRequestKey := krt.NewIndex(resolvedOwners, "jwks-request-key", func(resolved ResolvedOwner) []remotehttp.FetchKey {
		if resolved.Error != "" {
			return nil
		}
		return []remotehttp.FetchKey{resolved.Source.RequestKey}
	})
	requestGroups := ownersByRequestKey.AsCollection(append(inputs.KrtOpts.ToOptions("jwks/RequestGroups"), FetchKeyIndexCollectionFunc)...)
	sharedRequests := krt.NewCollection(requestGroups, func(kctx krt.HandlerContext, grouped krt.IndexObject[remotehttp.FetchKey, ResolvedOwner]) *SharedJwksRequest {
		return CollapseJwksSources(krt.IndexObject[remotehttp.FetchKey, JwksSource]{
			Key: grouped.Key,
			Objects: slices.Map(grouped.Objects, func(resolved ResolvedOwner) JwksSource {
				return resolved.Source
			}),
		})
	}, inputs.KrtOpts.ToOptions("jwks/Requests")...)

	return Collections{
		PolicyOwners:   policyOwners,
		BackendOwners:  backendOwners,
		Owners:         owners,
		ResolvedOwners: resolvedOwners,
		SharedRequests: sharedRequests,
	}
}

func CollapseJwksSources(grouped krt.IndexObject[remotehttp.FetchKey, JwksSource]) *SharedJwksRequest {
	if len(grouped.Objects) == 0 {
		return nil
	}

	sources := append([]JwksSource(nil), grouped.Objects...)
	sources = slices.SortFunc(sources, func(a, b JwksSource) int {
		return cmp.Compare(a.OwnerKey.String(), b.OwnerKey.String())
	})

	shared := SharedJwksRequest{
		RequestKey:     grouped.Key,
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
