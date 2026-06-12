package jwks

import (
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

type CollectionInputs struct {
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	Backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	Resolver             Resolver
	KrtOpts              krtutil.KrtOptions
}

type Collections struct {
	SharedRequests krt.Collection[SharedJwksRequest]
}

func NewCollections(inputs CollectionInputs) Collections {
	policyOwners := krt.NewManyCollection(inputs.AgentgatewayPolicies, func(kctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) []RemoteJwksOwner {
		return OwnersFromPolicy(policy)
	}, inputs.KrtOpts.ToOptions("jwks/policyOwners")...)
	backendOwners := krt.NewManyCollection(inputs.Backends, func(kctx krt.HandlerContext, backend *agentgateway.AgentgatewayBackend) []RemoteJwksOwner {
		return OwnersFromBackend(backend)
	}, inputs.KrtOpts.ToOptions("jwks/backendOwners")...)
	owners := krt.JoinCollection([]krt.Collection[RemoteJwksOwner]{policyOwners, backendOwners}, inputs.KrtOpts.ToOptions("jwks/owners")...)

	sources := krt.NewCollection(owners, func(kctx krt.HandlerContext, owner RemoteJwksOwner) *JwksSource {
		resolved, err := inputs.Resolver.ResolveOwner(kctx, owner)
		if err != nil {
			logger.Error("error generating remote jwks url or tls options", "error", err, "owner", owner.ID.String())
			return nil
		}

		return &JwksSource{
			OwnerKey: resolved.OwnerID,
			jwksRequestSpec: jwksRequestSpec{
				RequestKey:     resolved.RequestKey(),
				Target:         resolved.Target.Target,
				TLSConfig:      resolved.Target.TLSConfig,
				ProxyTLSConfig: resolved.Target.ProxyTLSConfig,
				TTL:            resolved.TTL,
			},
		}
	}, inputs.KrtOpts.ToOptions("jwks/sources")...)

	sharedRequests := remotecache.NewSharedRequestCollection(
		sources,
		"jwks",
		inputs.KrtOpts,
		func(source JwksSource) remotehttp.FetchKey { return source.RequestKey },
		collapseJwksSources,
	)

	return Collections{
		SharedRequests: sharedRequests,
	}
}

func collapseJwksSources(grouped krt.IndexObject[remotehttp.FetchKey, JwksSource]) *SharedJwksRequest {
	if len(grouped.Objects) == 0 {
		return nil
	}
	primary, minTTL := remotecache.CollapseSources(grouped.Objects,
		func(s JwksSource) string { return s.OwnerKey.String() },
		func(s JwksSource) time.Duration { return s.TTL })
	return &SharedJwksRequest{jwksRequestSpec{
		RequestKey:     grouped.Key,
		Target:         primary.Target,
		TLSConfig:      primary.TLSConfig,
		ProxyTLSConfig: primary.ProxyTLSConfig,
		TTL:            minTTL,
	}}
}
