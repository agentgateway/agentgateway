package translator

import (
	"testing"

	"github.com/stretchr/testify/require"
	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type attachmentTestResolver struct {
	parent *ParentInfo
}

func (r attachmentTestResolver) ParentsFor(krt.HandlerContext, utils.TypedNamespacedName) []*ParentInfo {
	return []*ParentInfo{r.parent}
}

func TestRouteAttachmentParentIdentity(t *testing.T) {
	gateway := types.NamespacedName{Namespace: "default", Name: "gateway"}
	service := types.NamespacedName{Namespace: "default", Name: "service"}
	gatewayGroup := new(gwv1.Group(wellknown.GatewayGVK.Group))
	serviceGroup := new(gwv1.Group(wellknown.ServiceGVK.Group))
	serviceEntryGroup := new(gwv1.Group(wellknown.ServiceEntryGVK.Group))
	extensionGroup := new(gwv1.Group("test.example"))
	for _, tt := range []struct {
		name       string
		group      *gwv1.Group
		kind       string
		serviceKey *types.NamespacedName
		gateway    types.NamespacedName
		wantKind   string
		wantName   string
	}{
		{name: "extension", group: extensionGroup, kind: "TestListenerResource", gateway: gateway, wantKind: "TestListenerResource", wantName: "parent"},
		{name: "listener set", group: gatewayGroup, kind: "ListenerSet", gateway: gateway, wantKind: "ListenerSet", wantName: "parent"},
		{name: "gateway", group: gatewayGroup, kind: "Gateway", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "gateway default group", kind: "Gateway", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "gateway default group and kind", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "listener set default group", kind: "ListenerSet", gateway: gateway, wantKind: "ListenerSet", wantName: "parent"},
		{name: "service", group: serviceGroup, kind: "Service", serviceKey: &service, gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "service entry", group: serviceEntryGroup, kind: "ServiceEntry", serviceKey: &service, gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "service-backed extension", group: extensionGroup, kind: "TestServiceResource", serviceKey: &service, gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "mesh service", group: serviceGroup, kind: "Service", serviceKey: &service},
		{name: "mesh extension", group: extensionGroup, kind: "TestListenerResource"},
		{name: "service without service key", group: serviceGroup, kind: "Service", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "service entry without service key", group: serviceEntryGroup, kind: "ServiceEntry", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "extension Gateway", group: extensionGroup, kind: "Gateway", gateway: gateway, wantKind: "Gateway", wantName: "parent"},
		{name: "extension Service", group: extensionGroup, kind: "Service", gateway: gateway, wantKind: "Service", wantName: "parent"},
		{name: "extension ServiceEntry", group: extensionGroup, kind: "ServiceEntry", gateway: gateway, wantKind: "ServiceEntry", wantName: "parent"},
		{name: "extension ListenerSet", group: extensionGroup, kind: "ListenerSet", gateway: gateway, wantKind: "ListenerSet", wantName: "parent"},
		{name: "service-backed extension Gateway", group: extensionGroup, kind: "Gateway", serviceKey: &service, gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "listener set without gateway", group: gatewayGroup, kind: "ListenerSet", wantKind: "ListenerSet", wantName: "parent"},
		{name: "extension ListenerSet without gateway", group: extensionGroup, kind: "ListenerSet"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			stop := test.NewStop(t)
			opts := krtutil.NewKrtOptions(stop, nil)
			var kind *gwv1.Kind
			if tt.kind != "" {
				kind = new(gwv1.Kind(tt.kind))
			}
			ref := gwv1.ParentReference{Group: tt.group, Kind: kind, Name: "parent"}
			route := &gwv1.HTTPRoute{
				Namespace: "default", Name: "route",
				Spec: gwv1.HTTPRouteSpec{CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{ref}}},
			}
			inputs := RouteContextInputs{
				Services:       krt.NewStaticCollection(nil, []*corev1.Service{{Namespace: "default", Name: "parent"}}),
				ServiceEntries: krt.NewStaticCollection(nil, []*networkingclient.ServiceEntry{{Namespace: "default", Name: "parent"}}),
				RouteParents: attachmentTestResolver{parent: &ParentInfo{
					ParentGateway: tt.gateway, ServiceKey: tt.serviceKey, ListenerKey: "listener", SectionName: "http",
					AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
				}},
				References: plugins.ReferenceTypes{AllowedParentReferences: sets.New(NormalizeReference(ref.Group, ref.Kind, wellknown.GatewayGVK.GroupKind()))},
			}
			attachments := gatewayRouteAttachmentCollection(inputs, krt.NewStaticCollection(nil, []*gwv1.HTTPRoute{route}), wellknown.HTTPRouteGVK, opts)
			require.True(t, attachments.WaitUntilSynced(stop))
			if tt.wantKind == "" {
				require.Empty(t, attachments.List())
				return
			}
			require.Len(t, attachments.List(), 1)
			got := attachments.List()[0]
			require.Equal(t, utils.TypedNamespacedName{Kind: tt.wantKind, Namespace: "default", Name: tt.wantName}, got.To)
			require.Equal(t, tt.gateway, got.Gateway)
			require.Equal(t, "http", got.ListenerName)
		})
	}
}
