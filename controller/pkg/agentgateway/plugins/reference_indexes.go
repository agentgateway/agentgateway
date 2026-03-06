package plugins

import (
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"
	"k8s.io/apimachinery/pkg/types"
)

// BuildReferenceIndex builds a set of indexes that can lookup objects through various means.
// For example, lookup associated Gateways for a Backend.
func BuildReferenceIndex(
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend],
	attachments krt.IndexCollection[translator.TypedResource, *translator.RouteAttachment],
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
	attachments krt.IndexCollection[translator.TypedResource, *translator.RouteAttachment]
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
	case wellknown.HTTPRouteGVK.Kind, wellknown.GRPCRouteGVK.Kind, wellknown.TCPRouteGVK.Kind, wellknown.GRPCRouteGVK.Kind:
		//krt.Fetch(ctx, p.attachments, krt.FilterKey(translator.TypedResource{})
	}
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

func (p ReferenceIndex) LookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	return p.LookupGatewaysForTarget(ctx, object)
}
