package plugins

import (
	"context"

	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
)

// AgwResourceStatusSyncHandler defines a function that handles status syncing for a specific resource type in AgentGateway
type AgwResourceStatusSyncHandler func(ctx context.Context, client apiclient.Client, namespacedName types.NamespacedName, status any) error

type PolicyPluginInput struct {
	Ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend]
}

type PolicyPlugin struct {
	Build func(PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy])
}

// ApplyPolicies extracts all policies from the collection
func (p *PolicyPlugin) ApplyPolicies(inputs PolicyPluginInput) (krt.Collection[AgwPolicy], krt.StatusCollection[controllers.Object, any]) {
	status, col := p.Build(inputs)
	return col, status
}

// AgwPolicy wraps an Agw policy for collection handling
type AgwPolicy struct {
	Policy *api.Policy
	// TODO: track errors per policy
}

func (p AgwPolicy) Equals(in AgwPolicy) bool {
	return protoconv.Equals(p.Policy, in.Policy)
}

func (p AgwPolicy) ResourceName() string {
	return p.Policy.Key
}

type AddResourcesPlugin struct {
	Binds     krt.Collection[ir.AgwResource]
	Listeners krt.Collection[ir.AgwResource]
	Routes    krt.Collection[ir.AgwResource]
}

// AddBinds extracts all bind resources from the collection
func (p *AddResourcesPlugin) AddBinds() krt.Collection[ir.AgwResource] {
	return p.Binds
}

// AddListeners extracts all routes resources from the collection
func (p *AddResourcesPlugin) AddListeners() krt.Collection[ir.AgwResource] {
	return p.Listeners
}

// AddRoutes extracts all routes resources from the collection
func (p *AddResourcesPlugin) AddRoutes() krt.Collection[ir.AgwResource] {
	return p.Routes
}

func ResourceName[T config.Namer](o T) *api.ResourceName {
	return &api.ResourceName{
		Namespace: o.GetNamespace(),
		Name:      o.GetName(),
	}
}

func TypedResourceName[T config.Namer](typ string, o T) *api.TypedResourceName {
	return &api.TypedResourceName{
		Kind:      typ,
		Namespace: o.GetNamespace(),
		Name:      o.GetName(),
	}
}

func TypedResourceFromName(typ string, o types.NamespacedName) *api.TypedResourceName {
	return &api.TypedResourceName{
		Kind:      typ,
		Namespace: o.Namespace,
		Name:      o.Name,
	}
}
