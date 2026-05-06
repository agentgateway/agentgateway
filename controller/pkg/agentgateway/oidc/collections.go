package oidc

import (
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

var collectionsLogger = logging.New("oidc_collections")

type CollectionInputs struct {
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	KrtOpts              krtutil.KrtOptions
}

type Collections struct {
	SharedRequests krt.Collection[SharedOidcRequest]
}

func NewCollections(inputs CollectionInputs) Collections {
	sources := krt.NewManyCollection(inputs.AgentgatewayPolicies, func(ctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) []OidcSource {
		owners := OwnersFromPolicy(policy)
		if len(owners) == 0 {
			return nil
		}

		sources := make([]OidcSource, 0, len(owners))
		for _, owner := range owners {
			discoveryURL, err := OidcDiscoveryURL(owner.Config.IssuerURL)
			if err != nil {
				collectionsLogger.Warn("skipping OIDC source with invalid issuer URL", "owner", owner.ID.String(), "issuerURL", owner.Config.IssuerURL, "error", err)
				continue
			}
			target := remotehttp.FetchTarget{URL: discoveryURL}
			sources = append(sources, OidcSource{
				OwnerKey:       owner.ID,
				RequestKey:     oidcRequestKey(target, owner.Config.IssuerURL),
				ExpectedIssuer: owner.Config.IssuerURL,
				Target:         target,
				TTL:            owner.TTL,
			})
		}
		return sources
	}, inputs.KrtOpts.ToOptions("oidc/sources")...)

	sourcesByRequestKey := krt.NewIndex(sources, "oidc-request-key", func(source OidcSource) []remotehttp.FetchKey {
		return []remotehttp.FetchKey{source.RequestKey}
	})
	requestGroups := sourcesByRequestKey.AsCollection(append(inputs.KrtOpts.ToOptions("oidc/requestGroups"), remotecache.FetchKeyIndexCollectionOption)...)

	sharedRequests := krt.NewCollection(requestGroups, func(ctx krt.HandlerContext, grouped krt.IndexObject[remotehttp.FetchKey, OidcSource]) *SharedOidcRequest {
		return collapseOidcSources(grouped)
	}, inputs.KrtOpts.ToOptions("oidc/sharedRequests")...)

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
	return &SharedOidcRequest{
		RequestKey:     grouped.Key,
		ExpectedIssuer: primary.ExpectedIssuer,
		Target:         primary.Target,
		TTL:            minTTL,
	}
}
