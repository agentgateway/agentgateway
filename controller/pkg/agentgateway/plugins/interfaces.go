package plugins

import (
	"sort"

	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

type PolicyPluginInput struct {
	References ReferenceIndex
}

type BackendPlugin struct {
	Build           func(PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[ir.AgwResource])
	BuildReferences func() krt.Collection[*PolicyAttachment]
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
	Binds            krt.Collection[ir.AgwResource]
	Listeners        krt.Collection[ir.AgwResource]
	Routes           krt.Collection[ir.AgwResource]
	AncestorBackends krt.Collection[*utils.AncestorBackend]
	GatewayStatuses  krt.StatusCollection[*gwv1.Gateway, gwv1.GatewayStatus]
	// ParentResolvers contribute additional parent resolution logic to the
	// main route pipeline.
	ParentResolvers []ParentResolver
}

// ParentInfo holds info about a "Parent" - something that can be referenced as a ParentRef in the API.
type ParentInfo struct {
	ParentGateway          types.NamespacedName
	ParentGatewayClassName string
	// ListenerKey is the internal key of the listener resource created for this parent.
	ListenerKey string
	// ServiceKey (optionally) links a parent reference to an individual Service.
	ServiceKey *types.NamespacedName
	// AllowedKinds indicates which kinds can be admitted by this Parent.
	AllowedKinds []gwv1.RouteGroupKind
	// Hostnames that must match to reference the Parent. Format is ns/hostname.
	Hostnames []string
	// OriginalHostname is the unprocessed form of Hostnames; how it appeared in users' config.
	OriginalHostname string
	// CreationTimestamp is used in determining listener precedence.
	CreationTimestamp metav1.Time

	SectionName    gwv1.SectionName
	Port           gwv1.PortNumber
	Protocol       gwv1.ProtocolType
	TLSPassthrough bool
}

func (g ParentInfo) Equals(other ParentInfo) bool {
	return g.ParentGateway == other.ParentGateway &&
		g.ParentGatewayClassName == other.ParentGatewayClassName &&
		g.ListenerKey == other.ListenerKey &&
		ptr.Equal(g.ServiceKey, other.ServiceKey) &&
		g.OriginalHostname == other.OriginalHostname &&
		g.SectionName == other.SectionName &&
		g.Port == other.Port &&
		g.Protocol == other.Protocol &&
		g.TLSPassthrough == other.TLSPassthrough &&
		g.CreationTimestamp == other.CreationTimestamp &&
		slices.EqualFunc(g.AllowedKinds, other.AllowedKinds, func(a, b gwv1.RouteGroupKind) bool {
			return a.Kind == b.Kind && ptr.Equal(a.Group, b.Group)
		}) &&
		slices.Equal(g.Hostnames, other.Hostnames)
}

// ParentResolver resolves parent references for routes.
type ParentResolver interface {
	ParentsFor(ctx krt.HandlerContext, pk utils.TypedNamespacedName) []*ParentInfo
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
