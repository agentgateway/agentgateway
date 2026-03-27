package translator

import (
	"fmt"

	wellknown2 "github.com/agentgateway/agentgateway/controller/pkg/wellknown"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// Reference stores a reference to a namespaced GVK, as used by ReferencePolicy
type Reference struct {
	Kind      schema.GroupVersionKind
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
func ReferenceGrantsCollection(referenceGrants krt.Collection[*gwv1b1.ReferenceGrant], krtopts krtutil.KrtOptions) krt.Collection[ReferenceGrant] {
	return krt.NewManyCollection(referenceGrants, func(ctx krt.HandlerContext, obj *gwv1b1.ReferenceGrant) []ReferenceGrant {
		rp := obj.Spec
		results := make([]ReferenceGrant, 0, len(rp.From)*len(rp.To))
		for _, from := range rp.From {
			fromKey := Reference{
				Namespace: from.Namespace,
			}
			if string(from.Group) == wellknown2.GatewayGVK.Group && string(from.Kind) == wellknown2.GatewayKind {
				fromKey.Kind = wellknown2.GatewayGVK
			} else if string(from.Group) == wellknown2.HTTPRouteGVK.Group && string(from.Kind) == wellknown2.HTTPRouteKind {
				fromKey.Kind = wellknown2.HTTPRouteGVK
			} else if string(from.Group) == wellknown2.GRPCRouteGVK.Group && string(from.Kind) == wellknown2.GRPCRouteKind {
				fromKey.Kind = wellknown2.GRPCRouteGVK
			} else if string(from.Group) == wellknown2.TLSRouteGVK.Group && string(from.Kind) == wellknown2.TLSRouteKind {
				fromKey.Kind = wellknown2.TLSRouteGVK
			} else if string(from.Group) == wellknown2.TCPRouteGVK.Group && string(from.Kind) == wellknown2.TCPRouteKind {
				fromKey.Kind = wellknown2.TCPRouteGVK
			} else if string(from.Group) == wellknown2.ListenerSetGVK.Group && string(from.Kind) == wellknown2.ListenerSetKind {
				fromKey.Kind = wellknown2.ListenerSetGVK
			} else {
				// Not supported type. Not an error; may be for another controller
				continue
			}
			for _, to := range rp.To {
				toKey := Reference{
					Namespace: gwv1b1.Namespace(obj.Namespace),
				}
				if to.Group == "" && string(to.Kind) == wellknown2.SecretGVK.Kind {
					toKey.Kind = wellknown2.SecretGVK
				} else if to.Group == "" && string(to.Kind) == wellknown2.ServiceKind {
					toKey.Kind = wellknown2.ServiceGVK
				} else {
					// Not supported type. Not an error; may be for another controller
					continue
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
	}, krtopts.ToOptions("ReferenceGrants")...)
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
	from := Reference{Kind: kind, Namespace: gwv1b1.Namespace(namespace)}
	to := Reference{Kind: wellknown2.SecretGVK, Namespace: gwv1b1.Namespace(secret.Namespace)}
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
	refKind schema.GroupVersionKind,
) bool {
	from := Reference{Kind: k, Namespace: gwv1b1.Namespace(routeNamespace)}
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
