package syncer_test

import (
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer"
)

// Native Gateway and ListenerSet behavior is covered by translator goldens.
// These tests exercise plugin wiring that the YAML harness cannot configure.
type testExtensionResolver struct {
	parents []*plugins.ParentInfo
	kind    string
}

func (r testExtensionResolver) ParentsFor(_ krt.HandlerContext, pk utils.TypedNamespacedName) []*plugins.ParentInfo {
	if pk.Kind != r.kind {
		return nil
	}
	return r.parents
}

func safetyGateway() *gwv1.Gateway {
	return &gwv1.Gateway{
		Name: "example", Namespace: "default",
		Spec: gwv1.GatewaySpec{
			GatewayClassName: "agentgateway",
			AllowedListeners: &gwv1.AllowedListeners{Namespaces: &gwv1.ListenerNamespaces{From: new(gwv1.NamespacesFromAll)}},
			Listeners:        []gwv1.Listener{{Name: "http", Port: 8080, Protocol: gwv1.HTTPProtocolType}},
		},
	}
}

func TestListenerSafetyAdditionalGatewayClassResolver(t *testing.T) {
	for _, tc := range []struct {
		name, returnedClass, listenerKey string
	}{
		{name: "additional class listener", returnedClass: "additional", listenerKey: "default/example.http"},
		{name: "delegated listener", returnedClass: "additional", listenerKey: "plugin/listener"},
		{name: "resolver supplied class", returnedClass: "agentgateway", listenerKey: "plugin/listener"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			gw := safetyGateway()
			gw.Spec.GatewayClassName = "additional"
			class := &gwv1.GatewayClass{Name: "additional", Spec: gwv1.GatewayClassSpec{ControllerName: "agentgateway.dev/agentgateway"}}
			route := &gwv1.HTTPRoute{
				Name: "additional", Namespace: "default",
				Spec: gwv1.HTTPRouteSpec{
					CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Name: "example"}}},
					Rules:           []gwv1.HTTPRouteRule{{}},
				},
			}
			ctx := testutils.BuildMockPolicyContext(t, []any{gatewayClassYAML, class, route, gw, &corev1.Namespace{Name: "default"}})
			parent := &plugins.ParentInfo{
				ParentGateway: exampleGateway, ParentGatewayClassName: tc.returnedClass, ListenerKey: tc.listenerKey,
				Hostnames: []string{"default/*"}, AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
			}
			_, s := testutils.SyncerWithExtensions(t, ctx, nil, plugins.AgwPlugin{
				AddResourceExtension: &plugins.AddResourcesPlugin{ParentResolvers: []plugins.ParentResolver{testExtensionResolver{parents: []*plugins.ParentInfo{parent}, kind: "Gateway"}}},
			}, map[string]*deployer.GatewayClassInfo{"additional": {}})
			assert.Empty(t, listenerKeys(s))
			var routeKeys []string
			for _, resource := range s.Outputs.Resources.List() {
				if r := resource.Resource.GetRoute(); r != nil && r.ListenerKey == tc.listenerKey {
					routeKeys = append(routeKeys, r.ListenerKey)
				}
			}
			assert.Equal(t, []string{tc.listenerKey}, routeKeys)
		})
	}
}

func TestListenerSafetyServiceParentResolver(t *testing.T) {
	for _, listenerKey := range []string{"", "plugin/listener", "default/example.http"} {
		t.Run(listenerKey, func(t *testing.T) {
			service := &corev1.Service{Name: "service", Namespace: "default", Spec: corev1.ServiceSpec{Ports: []corev1.ServicePort{{Port: 80}}}}
			route := &gwv1.HTTPRoute{
				Name: "service-route", Namespace: "default",
				Spec: gwv1.HTTPRouteSpec{
					CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Group: new(gwv1.Group("")), Kind: new(gwv1.Kind("Service")), Name: "service", Port: new(gwv1.PortNumber(80))}}},
					Rules:           []gwv1.HTTPRouteRule{{}},
				},
			}
			parent := &plugins.ParentInfo{
				ParentGateway: exampleGateway, ListenerKey: listenerKey,
				ServiceKey: &types.NamespacedName{Namespace: service.Namespace, Name: service.Name}, ServicePorts: []gwv1.PortNumber{80},
				Hostnames: []string{"default/*"}, AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
			}
			ctx := testutils.BuildMockPolicyContext(t, []any{gatewayClassYAML, safetyGateway(), service, route, &corev1.Namespace{Name: "default"}})
			sq, s := testutils.SyncerWithExtensions(t, ctx, []string{"HTTPRoute"}, plugins.AgwPlugin{
				AddResourceExtension: &plugins.AddResourcesPlugin{ParentResolvers: []plugins.ParentResolver{testExtensionResolver{parents: []*plugins.ParentInfo{parent}, kind: "Service"}}},
			}, nil, syncer.WithBuildReferenceTypes(func(_ *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes {
				base.AllowedParentReferences.Insert(schema.GroupKind{Kind: "Service"})
				return base
			}))
			var serviceRoutes int
			for _, resource := range s.Outputs.Resources.List() {
				if r := resource.Resource.GetRoute(); r != nil {
					serviceRoutes++
					assert.Equal(t, listenerKey, r.ListenerKey)
					require.NotNil(t, r.ServiceKey)
				}
			}
			assert.Equal(t, 1, serviceRoutes)
			dump, err := json.Marshal(sq.Dump())
			require.NoError(t, err)
			var statuses []struct {
				Status gwv1.HTTPRouteStatus `json:"status"`
			}
			require.NoError(t, json.Unmarshal(dump, &statuses))
			require.Len(t, statuses, 1)
			require.Len(t, statuses[0].Status.Parents, 1)
			accepted := meta.FindStatusCondition(statuses[0].Status.Parents[0].Conditions, "Accepted")
			require.NotNil(t, accepted)
			assert.Equal(t, metav1.ConditionTrue, accepted.Status)
		})
	}
}

func TestListenerSafetyExtensionResolverAndDormantRoutes(t *testing.T) {
	for _, conflicted := range []bool{false, true} {
		name := "programmed"
		port := gwv1.PortNumber(8081)
		if conflicted {
			name = "conflicted"
			port = 8080
		}
		t.Run(name, func(t *testing.T) {
			for _, tc := range []struct {
				name, routeKind, hostname, reason string
				accepted                          bool
			}{
				{name: "accepted", routeKind: "HTTPRoute", hostname: "api.example.com", accepted: true, reason: "Accepted"},
				{name: "resolver restricts route kind", routeKind: "GRPCRoute", hostname: "api.example.com", reason: "NotAllowedByListeners"},
				{name: "resolver restricts hostname", routeKind: "HTTPRoute", hostname: "other.example.com", reason: "NoMatchingListenerHostname"},
			} {
				t.Run(tc.name, func(t *testing.T) {
					ls := contributedListenerSet("extension", "http", port, gwv1.HTTPProtocolType, false)
					ls.ParentInfo.Hostnames = []string{"default/*"}
					ls.ParentInfo.AllowedKinds = []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}, {Kind: "GRPCRoute"}}
					route := &gwv1.HTTPRoute{
						Name: "extension-route", Namespace: "default",
						Spec: gwv1.HTTPRouteSpec{
							CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Group: new(gwv1.Group("test.example.com")), Kind: new(gwv1.Kind("TestListenerSet")), Name: "extension", SectionName: new(gwv1.SectionName("http"))}}},
							Hostnames:       []gwv1.Hostname{gwv1.Hostname(tc.hostname)},
							Rules:           []gwv1.HTTPRouteRule{{}},
						},
					}
					// Resolver-owned attachment restrictions must not be replaced with
					// the contributed listener's broader permissions, even if it loses.
					parent := ls.ParentInfo
					parent.Hostnames = []string{"default/api.example.com"}
					parent.AllowedKinds = []gwv1.RouteGroupKind{{Kind: gwv1.Kind(tc.routeKind)}}
					ctx := testutils.BuildMockPolicyContext(t, []any{gatewayClassYAML, safetyGateway(), route, &corev1.Namespace{Name: "default"}})
					sq, s := testutils.SyncerWithExtensions(t, ctx, []string{"HTTPRoute"}, plugins.AgwPlugin{
						AddResourceExtension: &plugins.AddResourcesPlugin{ParentResolvers: []plugins.ParentResolver{nil, testExtensionResolver{parents: []*plugins.ParentInfo{&parent}, kind: "TestListenerSet"}}},
					}, nil,
						syncer.WithExtraListenerSets(func(_ *plugins.AgwCollections, opts krtutil.KrtOptions) krt.Collection[*translator.ListenerSet] {
							return krt.NewStaticCollection(nil, []*translator.ListenerSet{ls}, opts.ToOptions("test/ExtensionListeners")...)
						}),
						syncer.WithBuildReferenceTypes(func(_ *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes {
							base.AllowedParentReferences.Insert(schema.GroupKind{Group: "test.example.com", Kind: "TestListenerSet"})
							return base
						}),
					)
					assert.Empty(t, ls.Conflict)
					if conflicted {
						assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))
					} else {
						assert.ElementsMatch(t, []string{"default/example.http", ls.Name}, listenerKeys(s))
					}
					var routeKeys []string
					for _, resource := range s.Outputs.Resources.List() {
						if r := resource.Resource.GetRoute(); r != nil {
							routeKeys = append(routeKeys, r.ListenerKey)
						}
					}
					if tc.accepted {
						assert.Equal(t, []string{ls.Name}, routeKeys)
					} else {
						assert.Empty(t, routeKeys)
					}
					dump, err := json.Marshal(sq.Dump())
					require.NoError(t, err)
					var statuses []struct {
						Status gwv1.HTTPRouteStatus `json:"status"`
					}
					require.NoError(t, json.Unmarshal(dump, &statuses))
					require.Len(t, statuses, 1)
					require.Len(t, statuses[0].Status.Parents, 1)
					assert.Equal(t, "TestListenerSet", string(*statuses[0].Status.Parents[0].ParentRef.Kind))
					accepted := meta.FindStatusCondition(statuses[0].Status.Parents[0].Conditions, "Accepted")
					require.NotNil(t, accepted)
					want := metav1.ConditionFalse
					if tc.accepted {
						want = metav1.ConditionTrue
					}
					assert.Equal(t, want, accepted.Status)
					assert.Equal(t, tc.reason, accepted.Reason)
				})
			}
		})
	}
}
