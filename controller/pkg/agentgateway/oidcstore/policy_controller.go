package oidcstore

import (
	"context"
	"fmt"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/backendtransport"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

var polLogger = logging.New("oidc_provider_store_policy_controller")

type OIDCStorePolicyController struct {
	agw           *plugins.AgwCollections
	sources       krt.Collection[oidc.ProviderSource]
	sourceChanges chan oidc.ProviderSource
}

func NewOIDCStorePolicyController(agw *plugins.AgwCollections) *OIDCStorePolicyController {
	polLogger.Info("creating oidc provider store policy controller")
	return &OIDCStorePolicyController{
		agw:           agw,
		sourceChanges: make(chan oidc.ProviderSource),
	}
}

func (c *OIDCStorePolicyController) Init(ctx context.Context) {
	lookup := backendtransport.NewBackendTransportLookup(
		c.agw.ConfigMaps,
		c.agw.Services,
		c.agw.Backends,
		c.agw.AgentgatewayPolicies,
		c.agw.BackendTLSPolicies,
	)
	c.sources = krt.NewManyCollection(
		c.agw.AgentgatewayPolicies,
		func(kctx krt.HandlerContext, p *agentgateway.AgentgatewayPolicy) []oidc.ProviderSource {
			pctx := plugins.PolicyCtx{
				Krt:                    kctx,
				Collections:            c.agw,
				BackendTransportLookup: lookup,
			}
			sources := make([]oidc.ProviderSource, 0)
			seen := make(map[string]struct{})

			if p.Spec.Traffic != nil && p.Spec.Traffic.JWTAuthentication != nil {
				for _, provider := range p.Spec.Traffic.JWTAuthentication.Providers {
					if provider.JWKS.OIDC == nil {
						continue
					}
					source, err := plugins.BuildOIDCProviderSource(
						pctx,
						p.Name,
						p.Namespace,
						string(provider.Issuer),
						provider.JWKS.OIDC.BackendRef,
					)
					if err != nil {
						polLogger.Error("error building oidc provider source", "error", err, "policy", p.Name, "issuer", provider.Issuer)
						continue
					}
					if _, ok := seen[source.ResourceKey]; ok {
						continue
					}
					seen[source.ResourceKey] = struct{}{}
					// Emit one source per policy owner. The store is responsible for
					// collapsing owners that share the same fetched provider source.
					source.OwnerKey = policySourceOwnerKey(p.Name, p.Namespace)
					sources = append(sources, *source)
				}
			}

			if p.Spec.Traffic != nil && p.Spec.Traffic.OAuth2 != nil && p.Spec.Traffic.OAuth2.Issuer != nil {
				source, err := plugins.BuildOIDCProviderSource(
					pctx,
					p.Name,
					p.Namespace,
					string(*p.Spec.Traffic.OAuth2.Issuer),
					p.Spec.Traffic.OAuth2.BackendRef,
				)
				if err != nil {
					polLogger.Error("error building oauth2 oidc provider source", "error", err, "policy", p.Name, "issuer", *p.Spec.Traffic.OAuth2.Issuer)
				} else if _, ok := seen[source.ResourceKey]; !ok {
					seen[source.ResourceKey] = struct{}{}
					// Emit one source per policy owner. The store is responsible for
					// collapsing owners that share the same fetched provider source.
					source.OwnerKey = policySourceOwnerKey(p.Name, p.Namespace)
					sources = append(sources, *source)
				}
			}

			return sources
		},
		c.agw.KrtOpts.ToOptions("OIDCProviderSources")...,
	)
}

func (c *OIDCStorePolicyController) Start(ctx context.Context) error {
	polLogger.Info("starting oidc provider store policy controller")
	c.sources.Register(func(event krt.Event[oidc.ProviderSource]) {
		switch event.Event {
		case controllers.EventAdd, controllers.EventUpdate:
			c.sourceChanges <- *event.New
		case controllers.EventDelete:
			deleted := *event.Old
			deleted.Deleted = true
			c.sourceChanges <- deleted
		}
	})

	<-ctx.Done()
	return nil
}

func (c *OIDCStorePolicyController) NeedLeaderElection() bool {
	return true
}

func (c *OIDCStorePolicyController) SourceChanges() chan oidc.ProviderSource {
	return c.sourceChanges
}

func policySourceOwnerKey(name, namespace string) string {
	return fmt.Sprintf("%s/%s", namespace, name)
}
