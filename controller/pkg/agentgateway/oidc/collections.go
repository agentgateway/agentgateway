package oidc

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
	Resolver             Resolver
	KrtOpts              krtutil.KrtOptions
}

type Collections struct {
	SharedRequests krt.Collection[SharedOidcRequest]
}

func NewCollections(inputs CollectionInputs) Collections {
	sources := krt.NewCollection(inputs.AgentgatewayPolicies, func(kctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) *OidcSource {
		owner, ok := OwnerFromPolicy(policy)
		if !ok {
			return nil
		}
		resolved, err := inputs.Resolver.ResolveOwner(kctx, owner)
		if err != nil {
			logger.Warn("skipping OIDC source: cannot resolve discovery endpoint", "owner", owner.ID.String(), "issuerURL", owner.Config.IssuerURL, "error", err)
			return nil
		}
		return &OidcSource{
			OwnerKey: resolved.OwnerID,
			oidcRequestSpec: oidcRequestSpec{
				RequestKey:     resolved.RequestKey(),
				ExpectedIssuer: resolved.ExpectedIssuer,
				Target:         resolved.Target.Target,
				ViaBackendRef:  resolved.ViaBackendRef,
				TLSConfig:      resolved.Target.TLSConfig,
				ProxyTLSConfig: resolved.Target.ProxyTLSConfig,
				TTL:            resolved.TTL,
			},
		}
	}, inputs.KrtOpts.ToOptions("oidc/sources")...)

	sharedRequests := remotecache.NewSharedRequestCollection(
		sources,
		"oidc",
		inputs.KrtOpts,
		func(source OidcSource) remotehttp.FetchKey { return source.RequestKey },
		collapseOidcSources,
	)

	return Collections{
		SharedRequests: sharedRequests,
	}
}

func collapseOidcSources(grouped krt.IndexObject[remotehttp.FetchKey, OidcSource]) *SharedOidcRequest {
	if len(grouped.Objects) == 0 {
		return nil
	}
	primary, minTTL := remotecache.CollapseSources(grouped.Objects,
		func(s OidcSource) string { return s.OwnerKey.String() },
		func(s OidcSource) time.Duration { return s.TTL })
	return &SharedOidcRequest{oidcRequestSpec{
		RequestKey:     grouped.Key,
		ExpectedIssuer: primary.ExpectedIssuer,
		Target:         primary.Target,
		ViaBackendRef:  primary.ViaBackendRef,
		TLSConfig:      primary.TLSConfig,
		ProxyTLSConfig: primary.ProxyTLSConfig,
		TTL:            minTTL,
	}}
}
