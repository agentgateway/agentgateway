package agentjwksstore

import (
	"context"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/jwks_url"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

// JwksStorePolicyController watches AgentgatewayPolicies and Backends. When a resource containing
// new or updated remote jwks source is detected, an jwks store is notifed of an update.
type JwksStorePolicyController struct {
	agw            *plugins.AgwCollections
	jwks           krt.Collection[jwks.JwksSource]
	jwksChanges    chan jwks.JwksSource
	jwksURLBuilder jwks_url.JwksUrlBuilder
}

var polLogger = logging.New("jwks_store_policy_controller")

func NewJWKSStorePolicyController(agw *plugins.AgwCollections, jwksURLBuilder jwks_url.JwksUrlBuilder) *JwksStorePolicyController {
	polLogger.Info("creating jwks store policy controller")
	return &JwksStorePolicyController{
		agw:            agw,
		jwksChanges:    make(chan jwks.JwksSource),
		jwksURLBuilder: jwksURLBuilder,
	}
}

func (j *JwksStorePolicyController) Init(ctx context.Context) {
	// TODO JwksSource should be per-policy, i.e. the same jwks url for multiple policies should result in multiple JwksSources
	// Otherwise changes to one policy (removal for example) could result in disruption of traffic for other policies (while ConfigMaps are re-synced)
	j.jwks = krt.NewManyCollection(j.agw.AgentgatewayPolicies, func(kctx krt.HandlerContext, p *agentgateway.AgentgatewayPolicy) []jwks.JwksSource {
		pctx := plugins.PolicyCtx{Krt: kctx, Collections: j.agw}
		return collectPolicyJWKSSources(kctx, p, pctx, j.agw.Backends, j.buildJwksSource)
	}, j.agw.KrtOpts.ToOptions("JwksSources")...)
}

func collectPolicyJWKSSources(
	kctx krt.HandlerContext,
	p *agentgateway.AgentgatewayPolicy,
	pctx plugins.PolicyCtx,
	backends krt.Collection[*agentgateway.AgentgatewayBackend],
	buildJwksSource func(krt.HandlerContext, string, string, *agentgateway.RemoteJWKS) *jwks.JwksSource,
) []jwks.JwksSource {
	toret := make([]jwks.JwksSource, 0)

	// enqueue Traffic JWT providers (if present)
	if p.Spec.Traffic != nil && p.Spec.Traffic.JWTAuthentication != nil {
		for _, provider := range p.Spec.Traffic.JWTAuthentication.Providers {
			switch {
			case provider.JWKS.Remote != nil:
				if s := buildJwksSource(kctx, p.Name, p.Namespace, provider.JWKS.Remote); s != nil {
					toret = append(toret, *s)
				}
			case provider.JWKS.OIDC != nil:
				s, err := plugins.ResolveOIDCJWKSSource(pctx, p.Name, p.Namespace, string(provider.Issuer), provider.JWKS.OIDC.BackendRef)
				if err != nil {
					polLogger.Error("error resolving oidc jwks source", "error", err, "policy", p.Name, "issuer", provider.Issuer)
					continue
				}
				toret = append(toret, *s)
			}
		}
	}

	if p.Spec.Traffic != nil && p.Spec.Traffic.OAuth2 != nil && p.Spec.Traffic.OAuth2.Issuer != nil {
		s, err := plugins.ResolveOIDCJWKSSource(
			pctx,
			p.Name,
			p.Namespace,
			string(*p.Spec.Traffic.OAuth2.Issuer),
			p.Spec.Traffic.OAuth2.BackendRef,
		)
		if err != nil {
			polLogger.Error("error resolving oauth2 oidc jwks source", "error", err, "policy", p.Name, "issuer", *p.Spec.Traffic.OAuth2.Issuer)
		} else {
			toret = append(toret, *s)
		}
	}

	// enqueue Backend MCP authentication JWKS (if present)
	if p.Spec.Backend != nil && p.Spec.Backend.MCP != nil && p.Spec.Backend.MCP.Authentication != nil {
		if s := buildJwksSource(kctx, p.Name, p.Namespace, &p.Spec.Backend.MCP.Authentication.JWKS); s != nil {
			toret = append(toret, *s)
		}
	}

	for _, b := range krt.Fetch(kctx, backends) {
		if b.Spec.MCP == nil {
			// ignore non-mcp backend types
			continue
		}
		if b.Spec.Policies != nil && b.Spec.Policies.MCP != nil && b.Spec.Policies.MCP.Authentication != nil {
			if s := buildJwksSource(kctx, p.Name, p.Namespace, &b.Spec.Policies.MCP.Authentication.JWKS); s != nil {
				toret = append(toret, *s)
			}
		}
	}

	return toret
}

func (j *JwksStorePolicyController) Start(ctx context.Context) error {
	polLogger.Info("starting jwks store policy controller")
	j.jwks.Register(func(o krt.Event[jwks.JwksSource]) {
		switch o.Event {
		case controllers.EventAdd, controllers.EventUpdate:
			j.jwksChanges <- *o.New
		case controllers.EventDelete:
			deleted := *o.Old
			deleted.Deleted = true
			j.jwksChanges <- deleted
		}
	})

	<-ctx.Done()
	return nil
}

// runs on the leader only
func (j *JwksStorePolicyController) NeedLeaderElection() bool {
	return true
}

func (j *JwksStorePolicyController) JwksChanges() chan jwks.JwksSource {
	return j.jwksChanges
}

func (j *JwksStorePolicyController) buildJwksSource(krtctx krt.HandlerContext, policyName, defaultNS string, remoteProvider *agentgateway.RemoteJWKS) *jwks.JwksSource {
	jwksUrl, tlsConfig, err := j.jwksURLBuilder.BuildJwksUrlAndTlsConfig(krtctx, policyName, defaultNS, remoteProvider)
	if err != nil {
		polLogger.Error("error generating remote jwks url or tls options", "error", err)
		return nil
	}

	return &jwks.JwksSource{
		JwksURL:   jwksUrl,
		TlsConfig: tlsConfig,
		Ttl:       remoteProvider.CacheDuration.Duration,
	}
}
