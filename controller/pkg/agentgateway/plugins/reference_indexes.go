package plugins

import (
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"
	"k8s.io/apimachinery/pkg/types"
)

// BuildReferenceIndex builds a set of indexes that can lookup objects through various means.
// For example, lookup associated Gateways for a Backend.
func BuildReferenceIndex(ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend]) ReferenceIndex {
	return ReferenceIndex{
		ancestors: ancestors,
	}
}

type ReferenceIndex struct {
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend]
}

func (p ReferenceIndex) LookupGateways(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
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
