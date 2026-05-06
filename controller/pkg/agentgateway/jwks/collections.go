package jwks

import (
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

var collectionsLogger = logging.New("jwks_collections")

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
	}, inputs.KrtOpts.ToOptions("PolicyJwksOwners")...)
	backendOwners := krt.NewManyCollection(inputs.Backends, func(kctx krt.HandlerContext, backend *agentgateway.AgentgatewayBackend) []RemoteJwksOwner {
		return OwnersFromBackend(backend)
	}, inputs.KrtOpts.ToOptions("BackendJwksOwners")...)
	owners := krt.JoinCollection([]krt.Collection[RemoteJwksOwner]{policyOwners, backendOwners}, inputs.KrtOpts.ToOptions("JwksOwners")...)

	primarySources := krt.NewCollection(owners, func(kctx krt.HandlerContext, owner RemoteJwksOwner) *JwksSource {
		resolved, err := inputs.Resolver.ResolveOwner(kctx, owner)
		if err != nil {
			collectionsLogger.Error("error generating remote jwks url or tls options", "error", err, "owner", owner.ID.String())
			return nil
		}

		return &JwksSource{
			OwnerKey:       resolved.OwnerID,
			RequestKey:     resolved.Target.Key,
			Target:         resolved.Target.Target,
			TLSConfig:      resolved.Target.TLSConfig,
			ProxyTLSConfig: resolved.Target.ProxyTLSConfig,
			TTL:            resolved.TTL,
		}
	}, inputs.KrtOpts.ToOptions("JwksSources/primary")...)

	sourcesByRequestKey := krt.NewIndex(primarySources, "jwks-request-key", func(source JwksSource) []remotehttp.FetchKey {
		return []remotehttp.FetchKey{source.RequestKey}
	})
	requestGroups := sourcesByRequestKey.AsCollection(append(inputs.KrtOpts.ToOptions("JwksRequestGroups"), remotecache.FetchKeyIndexCollectionOption)...)
	sharedRequests := krt.NewCollection(requestGroups, func(kctx krt.HandlerContext, grouped krt.IndexObject[remotehttp.FetchKey, JwksSource]) *SharedJwksRequest {
		return collapseJwksSources(grouped)
	}, inputs.KrtOpts.ToOptions("JwksRequests")...)

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
	return &SharedJwksRequest{
		RequestKey:     grouped.Key,
		Target:         primary.Target,
		TLSConfig:      primary.TLSConfig,
		ProxyTLSConfig: primary.ProxyTLSConfig,
		TTL:            minTTL,
	}
}
