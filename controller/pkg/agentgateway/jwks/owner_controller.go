package jwks

import (
	"context"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/client-go/tools/cache"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// OwnerController watches AgentgatewayPolicies and Backends and publishes
// changes to the resolved JWKS owners tracked by the subsystem.
type OwnerController struct {
	inputs      OwnerControllerInputs
	apiClient   apiclient.Client
	owners      krt.Collection[RemoteJwksOwner]
	jwks        krt.Collection[JwksSource]
	jwksChanges chan JwksSource
	waitForSync []cache.InformerSynced
}

type OwnerControllerInputs struct {
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	Backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	Resolver             Resolver
	KrtOpts              krtutil.KrtOptions
}

var polLogger = logging.New("jwks_owner_controller")

func NewOwnerController(apiClient apiclient.Client, inputs OwnerControllerInputs) *OwnerController {
	polLogger.Info("creating jwks owner controller")
	return &OwnerController{
		inputs:      inputs,
		apiClient:   apiClient,
		jwksChanges: make(chan JwksSource, 64),
	}
}

func (j *OwnerController) Init(ctx context.Context) {
	policyOwners := krt.NewManyCollection(j.inputs.AgentgatewayPolicies, func(kctx krt.HandlerContext, p *agentgateway.AgentgatewayPolicy) []RemoteJwksOwner {
		return OwnersFromPolicy(p)
	}, j.inputs.KrtOpts.ToOptions("PolicyJwksSources")...)
	backendOwners := krt.NewManyCollection(j.inputs.Backends, func(kctx krt.HandlerContext, b *agentgateway.AgentgatewayBackend) []RemoteJwksOwner {
		return OwnersFromBackend(b)
	}, j.inputs.KrtOpts.ToOptions("BackendJwksSources")...)
	j.owners = krt.JoinCollection([]krt.Collection[RemoteJwksOwner]{policyOwners, backendOwners}, j.inputs.KrtOpts.ToOptions("JwksOwners")...)
	j.jwks = krt.NewCollection(j.owners, func(kctx krt.HandlerContext, owner RemoteJwksOwner) *JwksSource {
		return j.resolveOwner(kctx, owner)
	}, j.inputs.KrtOpts.ToOptions("ResolvedJwksOwners")...)

	j.waitForSync = []cache.InformerSynced{
		j.inputs.AgentgatewayPolicies.HasSynced,
		j.inputs.Backends.HasSynced,
	}
}

func (j *OwnerController) Start(ctx context.Context) error {
	polLogger.Info("waiting for cache to sync")
	j.apiClient.Core().WaitForCacheSync(
		"kube AgentgatewayPolicy syncer",
		ctx.Done(),
		j.waitForSync...,
	)

	polLogger.Info("starting jwks owner controller")
	j.jwks.Register(func(o krt.Event[JwksSource]) {
		switch o.Event {
		case controllers.EventAdd, controllers.EventUpdate:
			if o.New != nil {
				j.jwksChanges <- *o.New
			}
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
func (j *OwnerController) NeedLeaderElection() bool {
	return true
}

func (j *OwnerController) JwksChanges() chan JwksSource {
	return j.jwksChanges
}

func (j *OwnerController) resolveOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) *JwksSource {
	resolved, err := j.inputs.Resolver.ResolveOwner(krtctx, owner)
	if err != nil {
		polLogger.Error("error generating remote jwks url or tls options", "error", err)
		return nil
	}

	return &JwksSource{
		OwnerKey:   resolved.OwnerID,
		RequestKey: resolved.Endpoint.Key,
		Request:    resolved.Endpoint.Request,
		TLSConfig:  resolved.Endpoint.TLSConfig,
		TTL:        resolved.TTL,
	}
}
