package plugins

import (
	"context"
	"sort"

	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
)

// AgwResourceStatusSyncHandler defines a function that handles status syncing for a specific resource type in AgentGateway
type AgwResourceStatusSyncHandler func(ctx context.Context, client apiclient.Client, namespacedName types.NamespacedName, status any) error

type PolicyPluginInput struct {
	References ReferenceIndex
}

type PolicyPlugin struct {
	Build           func(PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy])
	BuildReferences func(input PolicyPluginInput) krt.Collection[*PolicyAttachment]
}

// ApplyPolicies extracts all policies from the collection
func (p *PolicyPlugin) ApplyPolicies(inputs PolicyPluginInput) (krt.Collection[AgwPolicy], krt.StatusCollection[controllers.Object, any], krt.Collection[*PolicyAttachment]) {
	status, col := p.Build(inputs)
	var refs krt.Collection[*PolicyAttachment]
	if p.BuildReferences != nil {
		refs = p.BuildReferences(inputs)
	}
	return col, status, refs
}

// AgwPolicy wraps an Agw policy for collection handling
type AgwPolicy struct {
	Gateway *types.NamespacedName
	Policy  *api.Policy
	// TODO: track errors per policy
}

func (p AgwPolicy) Equals(in AgwPolicy) bool {
	return ptr.Equal(p.Gateway, in.Gateway) && protoconv.Equals(p.Policy, in.Policy)
}

func (p AgwPolicy) ResourceName() string {
	return p.Gateway.String() + "/" + p.Policy.Key
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

func appendPolicyForGateways(policies []AgwPolicy, gatewayTargets []types.NamespacedName, policy *api.Policy) []AgwPolicy {
	sort.Slice(gatewayTargets, func(i, j int) bool {
		return gatewayTargets[i].String() < gatewayTargets[j].String()
	})
	for _, gatewayTarget := range gatewayTargets {
		policies = append(policies, AgwPolicy{
			Gateway: ptr.Of(gatewayTarget),
			Policy:  policy,
		})
	}
	return policies
}
