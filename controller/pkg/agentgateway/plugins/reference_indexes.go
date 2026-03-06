package plugins

import (
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"
	"k8s.io/apimachinery/pkg/types"
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
		ancestors:   ancestors,
		attachments: attachments,
	}
}

type ReferenceIndex struct {
	// Backend --> Gateway
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend]
	// Route --> Gateway
	attachments krt.IndexCollection[utils.TypedNamespacedName, *RouteAttachment]
	// Gateway --> Gateway: trivial, no collection needed
	// ListenerSet --> Gateway: NOT present; ListenerSet attachment not implemented (but really should be!) in AgentgatewayPolicy anyways
}

func (p ReferenceIndex) LookupGatewaysForTarget(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	if object.Kind == wellknown.GatewayGVK.Kind {
		return sets.New(object.NamespacedName)
	}
	switch object.Kind {
	case wellknown.GatewayGVK.Kind:
		// Trivial case
		return sets.New(object.NamespacedName)
	case wellknown.HTTPRouteGVK.Kind, wellknown.GRPCRouteGVK.Kind, wellknown.TCPRouteGVK.Kind, wellknown.TLSRouteGVK.Kind:
		gateways := sets.New[types.NamespacedName]()
		if p.ancestors == nil {
			return gateways
		}
		for _, indexed := range krt.Fetch(ctx, p.attachments, krt.FilterKey(object.String())) {
			for _, ancestor := range indexed.Objects {
				gateways.Insert(ancestor.Gateway)
			}
		}
		return gateways
	default:
		gateways := sets.New[types.NamespacedName]()
		if p.ancestors == nil {
			return gateways
		}
		for _, indexed := range krt.Fetch(ctx, p.ancestors, krt.FilterKey(object.String())) {
			for _, ancestor := range indexed.Objects {
				gateways.Insert(ancestor.Gateway)
			}
		}
		return gateways
	}
}

func (p ReferenceIndex) LookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	return p.LookupGatewaysForTarget(ctx, object)
}
