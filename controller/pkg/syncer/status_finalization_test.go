package syncer_test

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/test"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/status"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type finalizationTestStatus struct {
	AttachedRoutes int
	Conflicts      int
	Rejected       int
}

type finalizationTestResolver struct {
	candidates krt.Collection[*translator.ListenerSet]
}

func (r *finalizationTestResolver) ParentsFor(ctx krt.HandlerContext, key utils.TypedNamespacedName) []*plugins.ParentInfo {
	if key.Kind != "TestListenerResource" {
		return nil
	}
	var parents []*plugins.ParentInfo
	for _, candidate := range krt.Fetch(ctx, r.candidates) {
		if candidate.ParentObject.NamespacedName == key.NamespacedName {
			parents = append(parents, &candidate.ParentInfo)
		}
	}
	return parents
}

type finalizationTestQueue struct {
	mu     sync.Mutex
	writes map[status.Resource][]any
}

func (q *finalizationTestQueue) Run(context.Context) {}

func (q *finalizationTestQueue) Push(resource status.Resource, data any) {
	q.mu.Lock()
	defer q.mu.Unlock()
	q.writes[resource] = append(q.writes[resource], data)
}

func (q *finalizationTestQueue) statuses(resource status.Resource) []any {
	q.mu.Lock()
	defer q.mu.Unlock()
	return append([]any(nil), q.writes[resource]...)
}

func TestFinalStatusCollectionsWithoutContributions(t *testing.T) {
	ctx := testutils.BuildMockPolicyContext(t, []any{gatewayClassYAML, gatewayYAML})
	var inputs syncer.FinalStatusCollectionsConfig
	calls := 0
	_, s := testutils.SyncerWithOptions(t, ctx, nil, syncer.WithFinalStatusCollections(func(cfg syncer.FinalStatusCollectionsConfig) {
		calls++
		inputs = cfg
	}))
	require.Equal(t, 1, calls)
	require.Same(t, s.StatusCollections(), inputs.StatusCollections)
	require.Equal(t, wellknown.DefaultAgwControllerName, inputs.ControllerName)
	require.Len(t, inputs.GatewayListeners.List(), 1)
	require.Empty(t, inputs.RejectedListenerSets.List())
	require.Empty(t, inputs.RouteAttachments.List())
}

func TestFinalStatusCollectionsLifecycle(t *testing.T) {
	stop := test.NewStop(t)
	opts := krtutil.NewKrtOptions(stop, nil)
	ctx := testutils.BuildMockPolicyContext(t, []any{gatewayClassYAML, gatewayYAML})
	resource := &unstructured.Unstructured{Object: map[string]any{
		"apiVersion": "test.example/v1", "kind": "TestListenerResource",
		"metadata": map[string]any{"name": "extension", "namespace": "default"},
	}}
	objects := krt.NewMutableCollection(nil, []*unstructured.Unstructured{resource}, opts.ToOptions("test/ExtensionObjects")...)
	free := contributedListenerSet("extension", "http", 8081, gwv1.HTTPProtocolType, false)
	free.ParentInfo.AllowedKinds = []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}}
	loser := contributedListenerSet("extension", "loser", 8080, gwv1.TCPProtocolType, false)
	rejected := contributedListenerSet("invalid", "invalid", 8082, gwv1.HTTPProtocolType, false)
	rejected.ParentObject.Kind = "InvalidContribution"
	candidates := krt.NewMutableCollection(nil, []*translator.ListenerSet{free, loser, rejected}, opts.ToOptions("test/Candidates")...)
	resolver := &finalizationTestResolver{}
	extensionRoute := &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "extension-route"},
		Spec: gwv1.HTTPRouteSpec{CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{
			Group: ptr.Of(gwv1.Group("test.example")), Kind: ptr.Of(gwv1.Kind("TestListenerResource")), Name: "extension", SectionName: ptr.Of(gwv1.SectionName("http")),
		}}}},
	}
	gatewayRoute := &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "gateway-route"},
		Spec:       gwv1.HTTPRouteSpec{CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{Name: "example"}}}},
	}
	routes := krt.NewMutableCollection(nil, []*gwv1.HTTPRoute{extensionRoute, gatewayRoute}, opts.ToOptions("test/Routes")...)
	ctx.Collections.HTTPRoutes = routes.AsCollection()
	var initial, final krt.StatusCollection[*unstructured.Unstructured, finalizationTestStatus]
	var inputs syncer.FinalStatusCollectionsConfig
	var order []string
	s := syncer.NewAgwSyncer(wellknown.DefaultAgwControllerName, fake.NewClient(t), ctx.Collections,
		plugins.AgwPlugin{AddResourceExtension: &plugins.AddResourcesPlugin{ParentResolvers: []plugins.ParentResolver{resolver}}},
		nil, opts, nil,
		syncer.WithExtraListenerSets(func(*plugins.AgwCollections, krtutil.KrtOptions) krt.Collection[*translator.ListenerSet] {
			return candidates.AsCollection()
		}),
		syncer.WithBuildReferenceTypes(func(_ *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes {
			base.AllowedParentReferences.Insert(schema.GroupKind{Group: "test.example", Kind: "TestListenerResource"})
			return base
		}),
		syncer.WithCustomResourceCollections(func(cfg syncer.CustomResourceCollectionsConfig) {
			order = append(order, "early")
			resolver.candidates = cfg.ListenerSets
			initial = krt.NewCollection(objects.AsCollection(), func(_ krt.HandlerContext, obj *unstructured.Unstructured) *krt.ObjectWithStatus[*unstructured.Unstructured, finalizationTestStatus] {
				return &krt.ObjectWithStatus[*unstructured.Unstructured, finalizationTestStatus]{Obj: obj}
			}, cfg.KrtOpts.ToOptions("test/InitialStatus")...)
		}),
		syncer.WithFinalStatusCollections(nil),
		syncer.WithFinalStatusCollections(func(cfg syncer.FinalStatusCollectionsConfig) {
			order = append(order, "final")
			inputs = cfg
			require.NotNil(t, initial)
			require.Equal(t, wellknown.DefaultAgwControllerName, cfg.ControllerName)
			attachmentIndex := krt.NewIndex(cfg.RouteAttachments, "to", func(a *plugins.RouteAttachment) []utils.TypedNamespacedName { return []utils.TypedNamespacedName{a.To} })
			final = krt.NewCollection(initial, func(kctx krt.HandlerContext, i krt.ObjectWithStatus[*unstructured.Unstructured, finalizationTestStatus]) *krt.ObjectWithStatus[*unstructured.Unstructured, finalizationTestStatus] {
				desired := i.Status
				desired.AttachedRoutes = len(krt.Fetch(kctx, cfg.RouteAttachments, krt.FilterIndex(attachmentIndex, utils.TypedNamespacedName{Kind: "TestListenerResource", Namespace: i.Obj.GetNamespace(), Name: i.Obj.GetName()})))
				for _, listener := range krt.Fetch(kctx, cfg.GatewayListeners) {
					if listener.ParentObject.Name == i.Obj.GetName() && listener.Conflict != "" {
						desired.Conflicts++
					}
				}
				desired.Rejected = len(krt.Fetch(kctx, cfg.RejectedListenerSets))
				return &krt.ObjectWithStatus[*unstructured.Unstructured, finalizationTestStatus]{Obj: i.Obj, Status: desired}
			}, cfg.KrtOpts.ToOptions("test/FinalStatus")...)
			status.RegisterStatus(cfg.StatusCollections, final, func(*unstructured.Unstructured) finalizationTestStatus { return finalizationTestStatus{} })
		}),
		syncer.WithFinalStatusCollections(func(cfg syncer.FinalStatusCollectionsConfig) {
			order = append(order, "second")
			require.Equal(t, inputs.RouteAttachments, cfg.RouteAttachments)
		}),
	)
	require.Equal(t, []string{"early", "final", "second"}, order)
	require.True(t, final.WaitUntilSynced(stop))
	queue := &finalizationTestQueue{writes: map[status.Resource][]any{}}
	for _, registration := range s.StatusCollections().SetQueue(queue) {
		require.True(t, registration.WaitUntilSynced(stop))
	}
	check := func(want finalizationTestStatus) {
		t.Helper()
		require.Eventually(t, func() bool {
			statuses := final.List()
			return len(statuses) == 1 && statuses[0].Status == want
		}, 5*time.Second, time.Millisecond)
	}
	check(finalizationTestStatus{AttachedRoutes: 1, Conflicts: 1, Rejected: 1})
	require.Len(t, inputs.RejectedListenerSets.List(), 1)
	require.Equal(t, gwv1.ListenerSetReasonInvalid, inputs.RejectedListenerSets.List()[0].Reason)
	extensionKey := status.Resource{GroupVersionKind: schema.GroupVersionKind{Group: "test.example", Version: "v1", Kind: "TestListenerResource"}, NamespacedName: free.ParentObject.NamespacedName}
	require.Len(t, queue.statuses(extensionKey), 1)
	require.Equal(t, finalizationTestStatus{AttachedRoutes: 1, Conflicts: 1, Rejected: 1}, *queue.statuses(extensionKey)[0].(*finalizationTestStatus))
	gatewayKey := status.Resource{GroupVersionKind: wellknown.GatewayGVK, NamespacedName: exampleGateway}
	gwWrites := queue.statuses(gatewayKey)
	require.NotEmpty(t, gwWrites)
	require.Equal(t, int32(1), gwWrites[len(gwWrites)-1].(*gwv1.GatewayStatus).Listeners[0].AttachedRoutes)

	routes.Reset([]*gwv1.HTTPRoute{gatewayRoute})
	check(finalizationTestStatus{Conflicts: 1, Rejected: 1})
	routes.Reset([]*gwv1.HTTPRoute{extensionRoute, gatewayRoute})
	check(finalizationTestStatus{AttachedRoutes: 1, Conflicts: 1, Rejected: 1})
	candidates.Reset([]*translator.ListenerSet{free})
	check(finalizationTestStatus{AttachedRoutes: 1})
	candidates.Reset(nil)
	check(finalizationTestStatus{})
	candidates.Reset([]*translator.ListenerSet{free, loser, rejected})
	check(finalizationTestStatus{AttachedRoutes: 1, Conflicts: 1, Rejected: 1})
	objects.Reset(nil)
	require.Eventually(t, func() bool { return len(final.List()) == 0 }, 5*time.Second, time.Millisecond)
	objects.Reset([]*unstructured.Unstructured{resource})
	check(finalizationTestStatus{AttachedRoutes: 1, Conflicts: 1, Rejected: 1})
	require.Equal(t, []string{"early", "final", "second"}, order)
}
