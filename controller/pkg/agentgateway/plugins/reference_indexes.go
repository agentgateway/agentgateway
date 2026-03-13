package plugins

import (
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

type RouteAttachment struct {
	// Route
	From utils.TypedNamespacedName
	// Immediate parent (Gateway or ListenerSet)
	To           utils.TypedNamespacedName
	ListenerName string
	// Eventual parent (always Gateway)
	Gateway types.NamespacedName
}

func (r RouteAttachment) ResourceName() string {
	to := r.To.String()
	if r.To.Kind != wellknown.GatewayGVK.Kind {
		to += "/" + r.Gateway.String()
	}
	return r.From.Kind + "/" + r.From.NamespacedName.String() + "->" + to + "/" + r.ListenerName
}

func (r RouteAttachment) Equals(other RouteAttachment) bool {
	return r.From == other.From && r.To == other.To && r.ListenerName == other.ListenerName && r.Gateway == other.Gateway
}

// BuildReferenceIndex builds a set of indexes that can lookup objects through various means.
// For example, lookup associated Gateways for a Backend.
func BuildReferenceIndex(
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend],
	attachments krt.IndexCollection[utils.TypedNamespacedName, *RouteAttachment],
) ReferenceIndex {
	return ReferenceIndex{
		Ancestors:   ancestors,
		attachments: attachments,
	}
}

type PolicyAttachment struct {
	Target  utils.TypedNamespacedName
	Backend utils.TypedNamespacedName
	Source  utils.TypedNamespacedName
}

func (a PolicyAttachment) Equals(other PolicyAttachment) bool {
	return a.Target == other.Target && a.Backend == other.Backend && a.Source == other.Source
}

func (a PolicyAttachment) ResourceName() string {
	return a.Source.String() + "/" + a.Target.String() + "/" + a.Backend.String()
}

type ReferenceIndex struct {
	// Backend --> Gateway via Route
	Ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend]
	// Backend --> Target via Policy
	PolicyAttachments krt.IndexCollection[utils.TypedNamespacedName, *PolicyAttachment]
	// Route --> Gateway
	attachments krt.IndexCollection[utils.TypedNamespacedName, *RouteAttachment]
	// Gateway --> Gateway: trivial, no collection needed
	// ListenerSet --> Gateway: NOT present; ListenerSet attachment not implemented (but really should be!) in AgentgatewayPolicy anyways
}

func (p ReferenceIndex) LookupGatewaysForTarget(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	switch object.Kind {
	case wellknown.GatewayGVK.Kind:
		// Trivial case
		return sets.New(object.NamespacedName)
	case wellknown.HTTPRouteGVK.Kind, wellknown.GRPCRouteGVK.Kind, wellknown.TCPRouteGVK.Kind, wellknown.TLSRouteGVK.Kind:
		gateways := sets.New[types.NamespacedName]()
		for _, ancestor := range krt.FetchOne(ctx, p.attachments, krt.FilterKey(object.String())).Objects {
			gateways.Insert(ancestor.Gateway)
		}
		return gateways
	default:
		gateways := sets.New[types.NamespacedName]()
		for _, ancestor := range krt.FetchOne(ctx, p.Ancestors, krt.FilterKey(object.String())).Objects {
			gateways.Insert(ancestor.Gateway)
		}
		return gateways
	}
}

func (p ReferenceIndex) LookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	return p.lookupGatewaysForBackend(ctx, object, map[string]struct{}{})
}

func (p ReferenceIndex) lookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName, seen map[string]struct{}) sets.Set[types.NamespacedName] {
	key := object.String()
	if _, ok := seen[key]; ok {
		return sets.New[types.NamespacedName]()
	}
	seen[key] = struct{}{}

	base := p.LookupGatewaysForTarget(ctx, object)
	if p.PolicyAttachments == nil {
		return base
	}
	for _, pref := range krt.FetchOne(ctx, p.PolicyAttachments, krt.FilterKey(key)).Objects {
		base = base.Union(p.lookupGatewaysForBackend(ctx, pref.Target, seen))
	}
	return base
}

func (p ReferenceIndex) WithPolicyAttachments(references krt.IndexCollection[utils.TypedNamespacedName, *PolicyAttachment]) ReferenceIndex {
	p.PolicyAttachments = references
	return p
}
