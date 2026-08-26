package translator

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestChildAllowsParent(t *testing.T) {
	parent := resolvedBinding{Parent: types.NamespacedName{Namespace: "parent-ns", Name: "parent"}}
	matchingParentRef := httpRouteParentRef("parent-ns", "parent")
	nonMatchingParentRef := httpRouteParentRef("parent-ns", "other")

	tests := []struct {
		name       string
		child      *gwv1.HTTPRoute
		grants     []ReferenceGrant
		grantMode  apisettings.BackendRefGrantMode
		wantAllows bool
	}{
		{
			name:       "parentless child in same namespace",
			child:      httpRoute("parent-ns", "child"),
			wantAllows: true,
		},
		{
			name:  "parentless child in another namespace without grant",
			child: httpRoute("child-ns", "child"),
		},
		{
			name:       "parentless child in another namespace without grant when grants disabled",
			child:      httpRoute("child-ns", "child"),
			grantMode:  apisettings.BackendRefGrantModeNone,
			wantAllows: true,
		},
		{
			name:       "parentless child in another namespace with kind-wide grant",
			child:      httpRoute("child-ns", "child"),
			grants:     []ReferenceGrant{httpRouteGrant("")},
			wantAllows: true,
		},
		{
			name:       "parentless child in another namespace with named grant",
			child:      httpRoute("child-ns", "child"),
			grants:     []ReferenceGrant{httpRouteGrant("child")},
			wantAllows: true,
		},
		{
			name:   "named grant applies to actual child name",
			child:  httpRoute("child-ns", "other-child"),
			grants: []ReferenceGrant{httpRouteGrant("child")},
		},
		{
			name: "explicit matching parent does not need a grant",
			child: func() *gwv1.HTTPRoute {
				route := httpRoute("child-ns", "child")
				route.Spec.ParentRefs = []gwv1.ParentReference{matchingParentRef}
				return route
			}(),
			wantAllows: true,
		},
		{
			name: "grant does not override a nonmatching explicit parent",
			child: func() *gwv1.HTTPRoute {
				route := httpRoute("child-ns", "child")
				route.Spec.ParentRefs = []gwv1.ParentReference{nonMatchingParentRef}
				return route
			}(),
			grants: []ReferenceGrant{httpRouteGrant("")},
		},
		{
			name: "disabled grants do not override a nonmatching explicit parent",
			child: func() *gwv1.HTTPRoute {
				route := httpRoute("child-ns", "child")
				route.Spec.ParentRefs = []gwv1.ParentReference{nonMatchingParentRef}
				return route
			}(),
			grantMode: apisettings.BackendRefGrantModeNone,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			assert.Equal(t, tt.wantAllows, childAllowsParent(
				krt.TestingDummyContext{},
				tt.child,
				parent,
				buildTestGrants(t, tt.grants),
				tt.grantMode,
			))
		})
	}
}

func buildTestGrants(t *testing.T, grants []ReferenceGrant) ReferenceGrants {
	t.Helper()
	opts := krtutil.NewKrtOptions(t.Context().Done(), nil)
	collection := krt.NewStaticCollection(nil, grants, opts.ToOptions("TestReferenceGrants")...)
	return BuildReferenceGrants(collection)
}

func httpRoute(namespace, name string) *gwv1.HTTPRoute {
	return &gwv1.HTTPRoute{Namespace: namespace, Name: name}
}

func httpRouteParentRef(namespace, name string) gwv1.ParentReference {
	group := gwv1.Group(wellknown.GatewayGroup)
	kind := gwv1.Kind(wellknown.HTTPRouteKind)
	ns := gwv1.Namespace(namespace)
	return gwv1.ParentReference{
		Group:     &group,
		Kind:      &kind,
		Namespace: &ns,
		Name:      gwv1.ObjectName(name),
	}
}

func httpRouteGrant(childName string) ReferenceGrant {
	grant := ReferenceGrant{
		Source: types.NamespacedName{Namespace: "child-ns", Name: "grant-" + childName},
		From: Reference{
			Kind:      wellknown.HTTPRouteGVK.GroupKind(),
			Namespace: "parent-ns",
		},
		To: Reference{
			Kind:      wellknown.HTTPRouteGVK.GroupKind(),
			Namespace: "child-ns",
		},
		AllowedName: childName,
	}
	grant.AllowAll = childName == ""
	return grant
}
