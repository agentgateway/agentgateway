package syncer_test

import (
	"encoding/json"
	"slices"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/annotations"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer"
)

func safetyGateway(hostname string) *gwv1.Gateway {
	gw := &gwv1.Gateway{
		Name: "example", Namespace: "default", CreationTimestamp: metav1.NewTime(time.Date(2026, 2, 1, 0, 0, 0, 0, time.UTC)),
		Spec: gwv1.GatewaySpec{
			GatewayClassName: "agentgateway",
			AllowedListeners: &gwv1.AllowedListeners{Namespaces: &gwv1.ListenerNamespaces{From: new(gwv1.NamespacesFromAll)}},
			Listeners: []gwv1.Listener{{Name: "http", Port: 8080, Protocol: gwv1.HTTPProtocolType,
				AllowedRoutes: &gwv1.AllowedRoutes{Namespaces: &gwv1.RouteNamespaces{From: new(gwv1.NamespacesFromAll)}}}},
		},
	}
	if hostname != "" {
		gw.Spec.Listeners[0].Hostname = new(gwv1.Hostname(hostname))
	}
	return gw
}

func safetyListenerSet(namespace, name, hostname string) *gwv1.ListenerSet {
	ls := &gwv1.ListenerSet{
		Name: name, Namespace: namespace, CreationTimestamp: metav1.NewTime(time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)),
		Spec: gwv1.ListenerSetSpec{
			ParentRef: gwv1.ParentGatewayReference{Name: "example", Namespace: new(gwv1.Namespace("default"))},
			Listeners: []gwv1.ListenerEntry{{Name: "http", Port: 8080, Protocol: gwv1.HTTPProtocolType}},
		},
	}
	if hostname != "" {
		ls.Spec.Listeners[0].Hostname = new(gwv1.Hostname(hostname))
	}
	return ls
}

func safetyInputs(gw *gwv1.Gateway, sets ...*gwv1.ListenerSet) []any {
	inputs := []any{gatewayClassYAML, gw,
		&corev1.Namespace{Name: "default"},
		&corev1.Namespace{Name: "tenant"},
	}
	for _, ls := range sets {
		inputs = append(inputs, ls)
	}
	return inputs
}

func TestListenerSafetyConflicts(t *testing.T) {
	for _, tc := range []struct {
		name, hostname, reason string
		protocol               gwv1.ProtocolType
		internal               bool
	}{
		{name: "catch-all-different-route-permissions", reason: "HostnameConflict", protocol: gwv1.HTTPProtocolType},
		{name: "exact-different-route-permissions", hostname: "example.com", reason: "HostnameConflict", protocol: gwv1.HTTPProtocolType},
		{name: "wildcard-different-route-permissions", hostname: "*.example.com", reason: "HostnameConflict", protocol: gwv1.HTTPProtocolType},
		{name: "protocol", reason: "ProtocolConflict", protocol: gwv1.TCPProtocolType},
		{name: "bind-mode", reason: "BindModeConflict", protocol: gwv1.HTTPProtocolType, internal: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			gw := safetyGateway(tc.hostname)
			ls := safetyListenerSet("tenant", "older", tc.hostname)
			ls.Spec.Listeners[0].Protocol = tc.protocol
			if tc.internal {
				ls.Annotations = map[string]string{annotations.InternalPorts: "8080"}
			}
			ctx := testutils.BuildMockPolicyContext(t, safetyInputs(gw, ls))
			sq, s := testutils.Syncer(t, ctx, "ListenerSet")
			assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))
			assert.Empty(t, s.Outputs.RejectedListenerSets.List())
			dump, err := json.Marshal(sq.Dump())
			require.NoError(t, err)
			assert.Contains(t, string(dump), `"reason":"`+tc.reason+`"`)
		})
	}
}

func TestListenerSafetyDistinctHostnames(t *testing.T) {
	gw := safetyGateway("")
	wildcard := safetyListenerSet("tenant", "wildcard", "*.example.com")
	exact := safetyListenerSet("tenant", "exact", "api.example.com")
	ctx := testutils.BuildMockPolicyContext(t, safetyInputs(gw, wildcard, exact))
	_, s := testutils.Syncer(t, ctx)
	assert.ElementsMatch(t, []string{"default/example.http", "tenant/wildcard.http", "tenant/exact.http"}, listenerKeys(s))
}

func TestListenerSafetyGatewayConflictStatus(t *testing.T) {
	for _, tc := range []struct {
		name, reason string
		protocol     gwv1.ProtocolType
	}{
		{name: "hostname", reason: "HostnameConflict", protocol: gwv1.HTTPProtocolType},
		{name: "protocol", reason: "ProtocolConflict", protocol: gwv1.TCPProtocolType},
	} {
		t.Run(tc.name, func(t *testing.T) {
			gw := safetyGateway("")
			gw.Spec.Listeners = append(gw.Spec.Listeners, gw.Spec.Listeners[0])
			gw.Spec.Listeners[1].Name = "second"
			gw.Spec.Listeners[1].Protocol = tc.protocol
			ctx := testutils.BuildMockPolicyContext(t, safetyInputs(gw))
			gateways := krt.NewMutableCollection(nil, []*gwv1.Gateway{gw})
			ctx.Collections.Gateways = gateways.AsCollection()
			sq, s := testutils.Syncer(t, ctx, "Gateway")
			assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))
			readStatus := func() gwv1.GatewayStatus {
				dump, err := json.Marshal(sq.Dump())
				require.NoError(t, err)
				var statuses []struct {
					Status gwv1.GatewayStatus `json:"status"`
				}
				require.NoError(t, json.Unmarshal(dump, &statuses))
				require.Len(t, statuses, 1)
				return statuses[0].Status
			}
			status := readStatus()
			require.Len(t, status.Listeners, 2)
			for _, condition := range []string{"Conflicted", "Accepted", "Programmed"} {
				got := meta.FindStatusCondition(status.Listeners[1].Conditions, condition)
				require.NotNil(t, got)
				assert.Equal(t, tc.reason, got.Reason)
				want := metav1.ConditionFalse
				if condition == "Conflicted" {
					want = metav1.ConditionTrue
				}
				assert.Equal(t, want, got.Status)
			}
			assert.Equal(t, metav1.ConditionFalse, meta.FindStatusCondition(status.Listeners[0].Conditions, "Conflicted").Status)
			edit := gw.DeepCopy()
			edit.Generation++
			edit.Spec.Listeners[1].Port = 8081
			gateways.UpdateObject(edit)
			require.Eventually(t, func() bool {
				status := readStatus()
				conflicted := meta.FindStatusCondition(status.Listeners[1].Conditions, "Conflicted")
				return len(listenerKeys(s)) == 2 && conflicted.Status == metav1.ConditionFalse && conflicted.ObservedGeneration == edit.Generation
			}, 5*time.Second, 10*time.Millisecond)
			for _, condition := range []string{"Accepted", "Programmed"} {
				assert.Equal(t, metav1.ConditionTrue, meta.FindStatusCondition(readStatus().Listeners[1].Conditions, condition).Status)
			}
		})
	}
}

func TestListenerSafetySetTies(t *testing.T) {
	gw := safetyGateway("owner.example.com")
	a := safetyListenerSet("default", "a", "shared.example.com")
	b := safetyListenerSet("default", "b", "shared.example.com")
	tenant := safetyListenerSet("tenant", "a", "shared.example.com")
	a.Spec.Listeners = append(a.Spec.Listeners, a.Spec.Listeners[0])
	a.Spec.Listeners[1].Name = "second"
	ctx := testutils.BuildMockPolicyContext(t, safetyInputs(gw, tenant, b, a))
	_, s := testutils.Syncer(t, ctx)
	assert.ElementsMatch(t, []string{"default/example.http", "default/a.http"}, listenerKeys(s))
}

func TestListenerSafetyInvalidTLSWinner(t *testing.T) {
	gw := safetyGateway("api.example.com")
	gw.Spec.Listeners[0].Protocol = gwv1.HTTPSProtocolType
	gw.Spec.Listeners[0].TLS = &gwv1.ListenerTLSConfig{CertificateRefs: []gwv1.SecretObjectReference{{Name: "missing"}}}
	loser := safetyListenerSet("tenant", "older", "api.example.com")
	loser.Spec.Listeners[0].Protocol = gwv1.HTTPSProtocolType
	loser.Spec.Listeners[0].TLS = gw.Spec.Listeners[0].TLS.DeepCopy()
	fallback := safetyListenerSet("tenant", "fallback", "")
	fallback.Spec.Listeners[0].Protocol = gwv1.HTTPSProtocolType
	fallback.Spec.Listeners[0].TLS = gw.Spec.Listeners[0].TLS.DeepCopy()
	ctx := testutils.BuildMockPolicyContext(t, safetyInputs(gw, loser, fallback))
	_, s := testutils.Syncer(t, ctx)
	assert.ElementsMatch(t, []string{"default/example.http", "tenant/fallback.http"}, listenerKeys(s))
	for _, r := range s.Outputs.Resources.List() {
		if l := r.Resource.GetListener(); l != nil && l.Key == "default/example.http" {
			require.NotNil(t, l.Tls)
			assert.Equal(t, []byte("invalid"), l.Tls.Cert)
			assert.Equal(t, []byte("invalid"), l.Tls.PrivateKey)
		}
	}
}

type testExtensionResolver struct {
	parents []*plugins.ParentInfo
	kind    string
}

func TestListenerSafetyAdditionalGatewayClassResolver(t *testing.T) {
	gw := safetyGateway("api.example.com")
	gw.Spec.GatewayClassName = "additional"
	class := &gwv1.GatewayClass{Name: "additional", Spec: gwv1.GatewayClassSpec{ControllerName: "agentgateway.dev/agentgateway"}}
	route := &gwv1.HTTPRoute{
		Name: "additional", Namespace: "default",
		Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Name: "example"}}},
			Rules:           []gwv1.HTTPRouteRule{{}},
		},
	}
	ctx := testutils.BuildMockPolicyContext(t, append(safetyInputs(gw), class, route))
	parent := &plugins.ParentInfo{
		ParentGateway: exampleGateway, ParentGatewayClassName: "additional", ListenerKey: "default/example.http",
		Hostnames: []string{"default/*"}, AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
	}
	_, s := testutils.SyncerWithExtensions(t, ctx, nil, plugins.AgwPlugin{
		AddResourceExtension: &plugins.AddResourcesPlugin{ParentResolvers: []plugins.ParentResolver{testExtensionResolver{parents: []*plugins.ParentInfo{parent}, kind: "Gateway"}}},
	}, map[string]*deployer.GatewayClassInfo{"additional": {}},
	)
	assert.Empty(t, listenerKeys(s))
	var routeKeys []string
	for _, resource := range s.Outputs.Resources.List() {
		if r := resource.Resource.GetRoute(); r != nil {
			routeKeys = append(routeKeys, r.ListenerKey)
		}
	}
	assert.Equal(t, []string{"default/example.http"}, routeKeys)
}

func TestListenerSafetyDelegatedParentGatewayOwnership(t *testing.T) {
	for _, returnedClass := range []string{"agentgateway", "additional"} {
		t.Run(returnedClass, func(t *testing.T) {
			testListenerSafetyDelegatedParentGatewayOwnership(t, returnedClass)
		})
	}
}

func testListenerSafetyDelegatedParentGatewayOwnership(t *testing.T, returnedClass string) {
	gw := safetyGateway("")
	gw.Spec.GatewayClassName = "additional"
	gw.Spec.Listeners = nil
	class := &gwv1.GatewayClass{Name: "additional", Spec: gwv1.GatewayClassSpec{ControllerName: "agentgateway.dev/agentgateway"}}
	route := &gwv1.HTTPRoute{
		Name: "delegated", Namespace: "default",
		Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Name: "example"}}},
			Rules:           []gwv1.HTTPRouteRule{{}},
		},
	}
	parent := &plugins.ParentInfo{
		ParentGateway: exampleGateway, ParentGatewayClassName: returnedClass, ListenerKey: "plugin/listener",
		Hostnames: []string{"default/*"}, AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
	}
	ctx := testutils.BuildMockPolicyContext(t, append(safetyInputs(gw), class, route))
	gateways := krt.NewMutableCollection(nil, []*gwv1.Gateway{gw})
	ctx.Collections.Gateways = gateways.AsCollection()
	_, s := testutils.SyncerWithExtensions(t, ctx, nil, plugins.AgwPlugin{
		AddResourceExtension: &plugins.AddResourcesPlugin{ParentResolvers: []plugins.ParentResolver{testExtensionResolver{parents: []*plugins.ParentInfo{parent}, kind: "Gateway"}}},
	}, map[string]*deployer.GatewayClassInfo{"additional": {}})
	routeCount := func() int {
		count := 0
		for _, resource := range s.Outputs.Resources.List() {
			if r := resource.Resource.GetRoute(); r != nil && r.ListenerKey == parent.ListenerKey {
				count++
			}
		}
		return count
	}
	require.Equal(t, 1, routeCount())
	edit := gw.DeepCopy()
	edit.Spec.GatewayClassName = "agentgateway"
	gateways.UpdateObject(edit)
	require.Eventually(t, func() bool { return routeCount() == 0 }, 5*time.Second, 10*time.Millisecond)
	gateways.UpdateObject(gw)
	require.Eventually(t, func() bool { return routeCount() == 1 }, 5*time.Second, 10*time.Millisecond)
	gateways.DeleteObject("default/example")
	require.Eventually(t, func() bool { return routeCount() == 0 }, 5*time.Second, 10*time.Millisecond)
	gateways.UpdateObject(gw)
	require.Eventually(t, func() bool { return routeCount() == 1 }, 5*time.Second, 10*time.Millisecond)
}

func (r testExtensionResolver) ParentsFor(_ krt.HandlerContext, pk utils.TypedNamespacedName) []*plugins.ParentInfo {
	if pk.Kind != r.kind {
		return nil
	}
	return r.parents
}

func TestListenerSafetyServiceParentResolver(t *testing.T) {
	for _, key := range []string{"", "plugin/listener", "default/example.http"} {
		t.Run(key, func(t *testing.T) {
			testListenerSafetyServiceParentResolver(t, key)
		})
	}
}

func testListenerSafetyServiceParentResolver(t *testing.T, listenerKey string) {
	gw := safetyGateway("")
	service := &corev1.Service{Name: "service", Namespace: "default", Spec: corev1.ServiceSpec{Ports: []corev1.ServicePort{{Port: 80}}}}
	route := &gwv1.HTTPRoute{
		Name: "service-route", Namespace: "default",
		Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Group: new(gwv1.Group("")), Kind: new(gwv1.Kind("Service")), Name: "service", Port: new(gwv1.PortNumber(80))}}},
			Rules:           []gwv1.HTTPRouteRule{{}},
		},
	}
	parent := &plugins.ParentInfo{
		ParentGateway: exampleGateway,
		ListenerKey:   listenerKey,
		ServiceKey:    &types.NamespacedName{Namespace: service.Namespace, Name: service.Name},
		ServicePorts:  []gwv1.PortNumber{80},
		Hostnames:     []string{"default/*"},
		AllowedKinds:  []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
	}
	ctx := testutils.BuildMockPolicyContext(t, append(safetyInputs(gw), service, route))
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
}

func TestListenerSafetyExtensionResolverAndDormantRoutes(t *testing.T) {
	gw := safetyGateway("")
	ls := contributedListenerSet("extension", "http", 8080, gwv1.HTTPProtocolType, false)
	ls.ParentInfo.Hostnames = []string{"default/*"}
	ls.ParentInfo.AllowedKinds = []gwv1.RouteGroupKind{{Group: new(gwv1.Group("gateway.networking.k8s.io")), Kind: "HTTPRoute"}}
	route := &gwv1.HTTPRoute{
		Name: "dormant", Namespace: "default",
		Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Group: new(gwv1.Group("test.example.com")), Kind: new(gwv1.Kind("TestListenerSet")), Name: "extension", SectionName: new(gwv1.SectionName("http"))}}},
			Rules:           []gwv1.HTTPRouteRule{{}},
		},
	}
	inputs := append(safetyInputs(gw), route)
	ctx := testutils.BuildMockPolicyContext(t, inputs)
	stale := ls.ParentInfo
	stale.AllowedKinds = nil
	sq, s := testutils.SyncerWithExtensions(t, ctx, []string{"HTTPRoute"}, plugins.AgwPlugin{
		AddResourceExtension: &plugins.AddResourcesPlugin{ParentResolvers: []plugins.ParentResolver{nil, testExtensionResolver{parents: []*plugins.ParentInfo{&stale}, kind: "TestListenerSet"}}},
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
	assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))
	var routeKeys []string
	for _, resource := range s.Outputs.Resources.List() {
		if r := resource.Resource.GetRoute(); r != nil {
			routeKeys = append(routeKeys, r.ListenerKey)
		}
	}
	assert.Equal(t, []string{ls.Name}, routeKeys)
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
	assert.Equal(t, metav1.ConditionTrue, accepted.Status)
	assert.Equal(t, "Accepted", accepted.Reason)
}

func TestListenerSafetyRecovery(t *testing.T) {
	gw := safetyGateway("shared.example.com")
	ls := safetyListenerSet("tenant", "delegate", "shared.example.com")
	ctx := testutils.BuildMockPolicyContext(t, safetyInputs(gw, ls))
	gateways := krt.NewMutableCollection(nil, []*gwv1.Gateway{gw})
	ctx.Collections.Gateways = gateways.AsCollection()
	_, s := testutils.Syncer(t, ctx)
	assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))
	edit := gw.DeepCopy()
	edit.Spec.Listeners[0].Hostname = new(gwv1.Hostname("owner.example.com"))
	gateways.UpdateObject(edit)
	require.Eventually(t, func() bool { return len(listenerKeys(s)) == 2 }, time.Second*5, time.Millisecond*10)
	gateways.UpdateObject(gw)
	require.Eventually(t, func() bool { return len(listenerKeys(s)) == 1 }, time.Second*5, time.Millisecond*10)
	assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))

	edit = gw.DeepCopy()
	edit.Spec.Listeners = nil
	gateways.UpdateObject(edit)
	require.Eventually(t, func() bool {
		keys := listenerKeys(s)
		return len(keys) == 1 && keys[0] == "tenant/delegate.http"
	}, time.Second*5, time.Millisecond*10)
}

func TestListenerSafetySetWinnerRemoval(t *testing.T) {
	gw := safetyGateway("owner.example.com")
	a := safetyListenerSet("tenant", "a", "shared.example.com")
	b := safetyListenerSet("tenant", "b", "shared.example.com")
	ctx := testutils.BuildMockPolicyContext(t, safetyInputs(gw, a, b))
	sets := krt.NewMutableCollection(nil, []*gwv1.ListenerSet{a, b})
	ctx.Collections.ListenerSets = sets.AsCollection()
	_, s := testutils.Syncer(t, ctx)
	assert.ElementsMatch(t, []string{"default/example.http", "tenant/a.http"}, listenerKeys(s))
	sets.DeleteObject("tenant/a")
	require.Eventually(t, func() bool {
		keys := listenerKeys(s)
		return len(keys) == 2 && slices.Contains(keys, "default/example.http") && slices.Contains(keys, "tenant/b.http")
	}, time.Second*5, time.Millisecond*10)
	assert.ElementsMatch(t, []string{"default/example.http", "tenant/b.http"}, listenerKeys(s))
}

func TestListenerSafetyNativeDormantRouteStatus(t *testing.T) {
	for _, tc := range []struct {
		name     string
		internal bool
		protocol gwv1.ProtocolType
	}{
		{name: "hostname", protocol: gwv1.HTTPProtocolType},
		{name: "bind-mode", internal: true, protocol: gwv1.HTTPProtocolType},
		{name: "protocol", protocol: gwv1.HTTPSProtocolType},
	} {
		t.Run(tc.name, func(t *testing.T) {
			gw := safetyGateway("")
			ls := safetyListenerSet("tenant", "delegate", "")
			ls.Spec.Listeners[0].Protocol = tc.protocol
			if tc.protocol == gwv1.HTTPSProtocolType {
				ls.Spec.Listeners[0].TLS = &gwv1.ListenerTLSConfig{CertificateRefs: []gwv1.SecretObjectReference{{Name: "missing"}}}
			}
			if tc.internal {
				ls.Annotations = map[string]string{annotations.InternalPorts: "8080"}
			}
			route := &gwv1.HTTPRoute{
				Name: "dormant", Namespace: "tenant",
				Spec: gwv1.HTTPRouteSpec{
					CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Kind: new(gwv1.Kind("ListenerSet")), Name: "delegate"}}},
					Rules:           []gwv1.HTTPRouteRule{{}},
				},
			}
			ctx := testutils.BuildMockPolicyContext(t, append(safetyInputs(gw, ls), route))
			sq, s := testutils.Syncer(t, ctx, "HTTPRoute", "ListenerSet")
			assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))
			dump, err := json.Marshal(sq.Dump())
			require.NoError(t, err)
			var statuses []struct {
				Kind   string          `json:"kind"`
				Status json.RawMessage `json:"status"`
			}
			require.NoError(t, json.Unmarshal(dump, &statuses))
			require.Len(t, statuses, 2)
			for _, status := range statuses {
				switch status.Kind {
				case "ListenerSet":
					var st gwv1.ListenerSetStatus
					require.NoError(t, json.Unmarshal(status.Status, &st))
					require.Len(t, st.Listeners, 1)
					assert.EqualValues(t, 1, st.Listeners[0].AttachedRoutes)
					programmed := meta.FindStatusCondition(st.Listeners[0].Conditions, "Programmed")
					require.NotNil(t, programmed)
					assert.Equal(t, metav1.ConditionFalse, programmed.Status)
				case "HTTPRoute":
					var st gwv1.HTTPRouteStatus
					require.NoError(t, json.Unmarshal(status.Status, &st))
					require.Len(t, st.Parents, 1)
					accepted := meta.FindStatusCondition(st.Parents[0].Conditions, "Accepted")
					require.NotNil(t, accepted)
					assert.Equal(t, "Accepted", accepted.Reason)
					assert.Equal(t, metav1.ConditionTrue, accepted.Status)
				}
			}
		})
	}
}
