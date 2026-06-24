package translator

import (
	"fmt"

	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// Reference stores a reference to a namespaced GroupKind, as used by ReferenceGrant.
type Reference struct {
	Kind      schema.GroupKind
	Namespace gwv1b1.Namespace
}

func (refs Reference) String() string {
	return refs.Kind.String() + "/" + string(refs.Namespace)
}

type ReferencePair struct {
	To, From Reference
}

func (r ReferencePair) String() string {
	return fmt.Sprintf("%s->%s", r.To, r.From)
}

type ReferenceGrants struct {
	collection krt.Collection[ReferenceGrant]
	index      krt.Index[ReferencePair, ReferenceGrant]
}

// ReferenceGrantsCollection creates a collection of ReferenceGrant objects from a collection of ReferenceGrant objects.
func ReferenceGrantsCollection(
	referenceGrants krt.Collection[*gwv1b1.ReferenceGrant],
	knownFromReferences sets.Set[schema.GroupKind],
	knownToReferences sets.Set[schema.GroupKind],
	krtopts krtutil.KrtOptions,
) krt.Collection[ReferenceGrant] {
	return krt.NewManyCollection(referenceGrants, func(ctx krt.HandlerContext, obj *gwv1b1.ReferenceGrant) []ReferenceGrant {
		rp := obj.Spec
		results := make([]ReferenceGrant, 0, len(rp.From)*len(rp.To))
		for _, from := range rp.From {
			fromGK := schema.GroupKind{Group: string(from.Group), Kind: string(from.Kind)}
			if !knownFromReferences.Contains(fromGK) {
				// Not supported type. Not an error; may be for another controller.
				continue
			}
			fromKey := Reference{
				Kind:      fromGK,
				Namespace: from.Namespace,
			}
			for _, to := range rp.To {
				toGK := schema.GroupKind{Group: string(to.Group), Kind: string(to.Kind)}
				if !knownToReferences.Contains(toGK) {
					// Not supported type. Not an error; may be for another controller.
					continue
				}
				toKey := Reference{
					Kind:      toGK,
					Namespace: gwv1b1.Namespace(obj.Namespace),
				}
				rg := ReferenceGrant{
					Source:      config.NamespacedName(obj),
					From:        fromKey,
					To:          toKey,
					AllowAll:    false,
					AllowedName: "",
				}
				if to.Name != nil {
					rg.AllowedName = string(*to.Name)
				} else {
					rg.AllowAll = true
				}
				results = append(results, rg)
			}
		}
		return results
	}, krtopts.ToOptions("translator/ReferenceGrants")...)
}

// BuildReferenceGrants creates a ReferenceGrants object from a collection of ReferenceGrant objects.
func BuildReferenceGrants(collection krt.Collection[ReferenceGrant]) ReferenceGrants {
	idx := krt.NewIndex(collection, "refgrant", func(o ReferenceGrant) []ReferencePair {
		return []ReferencePair{{
			To:   o.To,
			From: o.From,
		}}
	})
	return ReferenceGrants{
		collection: collection,
		index:      idx,
	}
}

// ReferenceGrant stores a reference grant between two references
type ReferenceGrant struct {
	Source      types.NamespacedName
	From        Reference
	To          Reference
	AllowAll    bool
	AllowedName string
}

func (g ReferenceGrant) ResourceName() string {
	nameKey := "*"
	if !g.AllowAll {
		nameKey = g.AllowedName
	}
	return g.Source.String() + "/" + g.From.String() + "/" + g.To.String() + "/" + nameKey
}

// SecretAllowed checks if a secret is allowed to be used by a gateway
func (refs ReferenceGrants) SecretAllowed(ctx krt.HandlerContext, kind schema.GroupVersionKind, secret types.NamespacedName, namespace string) bool {
	from := Reference{Kind: kind.GroupKind(), Namespace: gwv1b1.Namespace(namespace)}
	to := Reference{Kind: wellknown.SecretGVK.GroupKind(), Namespace: gwv1b1.Namespace(secret.Namespace)}
	pair := ReferencePair{From: from, To: to}
	grants := krt.Fetch(ctx, refs.collection, krt.FilterIndex(refs.index, pair))
	for _, g := range grants {
		if g.AllowAll || g.AllowedName == secret.Name {
			return true
		}
	}
	return false
}

// BackendAllowed checks if a backend is allowed to be used by a route
func (refs ReferenceGrants) BackendAllowed(
	ctx krt.HandlerContext,
	k schema.GroupVersionKind,
	backendName gwv1b1.ObjectName,
	backendNamespace gwv1b1.Namespace,
	routeNamespace string,
	refKind schema.GroupKind,
	mode apisettings.BackendRefGrantMode,
) bool {
	if !mode.RequireRouteBackendGrant() {
		return true
	}
	if refKind == wellknown.HTTPRouteGVK.GroupKind() {
		// ReferenceGrant not required for route delegation
		return true
	}
	from := Reference{Kind: k.GroupKind(), Namespace: gwv1b1.Namespace(routeNamespace)}
	to := Reference{Kind: refKind, Namespace: backendNamespace}
	pair := ReferencePair{From: from, To: to}
	grants := krt.Fetch(ctx, refs.collection, krt.FilterIndex(refs.index, pair))
	for _, g := range grants {
		if g.AllowAll || g.AllowedName == string(backendName) {
			return true
		}
	}
	return false
}
