package syncer_test

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/annotations"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer"
)

func safetyGateway(hostname string) *gwv1.Gateway {
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{Name: "example", Namespace: "default", CreationTimestamp: metav1.NewTime(time.Date(2026, 2, 1, 0, 0, 0, 0, time.UTC))},
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
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: namespace, CreationTimestamp: metav1.NewTime(time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC))},
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
		&corev1.Namespace{ObjectMeta: metav1.ObjectMeta{Name: "default"}},
		&corev1.Namespace{ObjectMeta: metav1.ObjectMeta{Name: "tenant"}},
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
	parents translator.RouteParents
}

func (r testExtensionResolver) ParentsFor(ctx krt.HandlerContext, pk utils.TypedNamespacedName) []*plugins.ParentInfo {
	if pk.Kind != "TestListenerSet" {
		return nil
	}
	pk.Kind = "ListenerSet"
	return r.parents.ParentsFor(ctx, pk)
}

func TestListenerSafetyExtensionResolverAndDormantRoutes(t *testing.T) {
	gw := safetyGateway("")
	ls := contributedListenerSet("extension", "http", 8080, gwv1.HTTPProtocolType, false)
	ls.ParentInfo.Hostnames = []string{"default/*"}
	ls.ParentInfo.AllowedKinds = []gwv1.RouteGroupKind{{Group: new(gwv1.Group("gateway.networking.k8s.io")), Kind: "HTTPRoute"}}
	route := &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{Name: "dormant", Namespace: "default"},
		Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Group: new(gwv1.Group("test.example.com")), Kind: new(gwv1.Kind("TestListenerSet")), Name: "extension", SectionName: new(gwv1.SectionName("http"))}}},
			Rules:           []gwv1.HTTPRouteRule{{}},
		},
	}
	inputs := append(safetyInputs(gw), route)
	ctx := testutils.BuildMockPolicyContext(t, inputs)
	var finalListeners krt.Collection[*translator.GatewayListener]
	sq, s := testutils.SyncerWithOptions(t, ctx, []string{"HTTPRoute"},
		syncer.WithExtraListenerSets(func(_ *plugins.AgwCollections, opts krtutil.KrtOptions) krt.Collection[*translator.ListenerSet] {
			return krt.NewStaticCollection(nil, []*translator.ListenerSet{ls}, opts.ToOptions("test/ExtensionListeners")...)
		}),
		syncer.WithBuildReferenceTypes(func(_ *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes {
			base.AllowedParentReferences.Insert(schema.GroupKind{Group: "test.example.com", Kind: "TestListenerSet"})
			return base
		}),
		syncer.WithListenerParentResolver(func(listeners krt.Collection[*translator.GatewayListener], _ krtutil.KrtOptions) translator.ParentResolver {
			finalListeners = listeners
			return testExtensionResolver{parents: translator.BuildRouteParents(listeners)}
		}),
	)
	require.NotNil(t, finalListeners)
	listener := finalListeners.GetKey(ls.Name)
	require.NotNil(t, listener)
	assert.Equal(t, translator.ListenerConflict(translator.ListenerConflictHostname), (*listener).Conflict)
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
	assert.Equal(t, metav1.ConditionTrue, statuses[0].Status.Parents[0].Conditions[0].Status)
	assert.Equal(t, "Accepted", statuses[0].Status.Parents[0].Conditions[0].Reason)
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
		for _, key := range keys {
			if key == "tenant/a.http" {
				return false
			}
		}
		return len(keys) == 2
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
				ObjectMeta: metav1.ObjectMeta{Name: "dormant", Namespace: "tenant"},
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
					foundProgrammed := false
					for _, condition := range st.Listeners[0].Conditions {
						if condition.Type == "Programmed" {
							foundProgrammed = true
							assert.Equal(t, metav1.ConditionFalse, condition.Status)
						}
					}
					assert.True(t, foundProgrammed)
				case "HTTPRoute":
					var st gwv1.HTTPRouteStatus
					require.NoError(t, json.Unmarshal(status.Status, &st))
					require.Len(t, st.Parents, 1)
					assert.Equal(t, "Accepted", st.Parents[0].Conditions[0].Reason)
					assert.Equal(t, metav1.ConditionTrue, st.Parents[0].Conditions[0].Status)
				}
			}
		})
	}
}
