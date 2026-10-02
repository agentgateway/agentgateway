package translator

import (
	"testing"

	"github.com/stretchr/testify/require"
	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
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
	for _, tt := range []struct {
		name       string
		kind       string
		serviceKey *types.NamespacedName
		gateway    types.NamespacedName
		wantKind   string
		wantName   string
	}{
		{name: "extension", kind: "TestListenerResource", gateway: gateway, wantKind: "TestListenerResource", wantName: "parent"},
		{name: "listener set", kind: "ListenerSet", gateway: gateway, wantKind: "ListenerSet", wantName: "parent"},
		{name: "gateway", kind: "Gateway", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "service", kind: "Service", serviceKey: &service, gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "service entry", kind: "ServiceEntry", serviceKey: &service, gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "service-backed extension", kind: "TestServiceResource", serviceKey: &service, gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "mesh service", kind: "Service", serviceKey: &service},
		{name: "mesh extension", kind: "TestListenerResource"},
		{name: "service without service key", kind: "Service", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
		{name: "service entry without service key", kind: "ServiceEntry", gateway: gateway, wantKind: "Gateway", wantName: "gateway"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			stop := test.NewStop(t)
			opts := krtutil.NewKrtOptions(stop, nil)
			route := &gwv1.HTTPRoute{
				Namespace: "default", Name: "route",
				Spec: gwv1.HTTPRouteSpec{CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: []gwv1.ParentReference{{
					Group: ptr.Of(gwv1.Group("test.example")), Kind: new(gwv1.Kind(tt.kind)), Name: "parent",
				}}}},
			}
			inputs := RouteContextInputs{
				Services:       krt.NewStaticCollection(nil, []*corev1.Service{{Namespace: "default", Name: "parent"}}),
				ServiceEntries: krt.NewStaticCollection(nil, []*networkingclient.ServiceEntry{{Namespace: "default", Name: "parent"}}),
				RouteParents: attachmentTestResolver{parent: &ParentInfo{
					ParentGateway: tt.gateway, ServiceKey: tt.serviceKey, ListenerKey: "listener", SectionName: "http",
					AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
				}},
				References: plugins.ReferenceTypes{AllowedParentReferences: sets.New(schema.GroupKind{Group: "test.example", Kind: tt.kind})},
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
