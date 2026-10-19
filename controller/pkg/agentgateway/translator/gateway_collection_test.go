package translator

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

func TestListenerConflictCopyOnWrite(t *testing.T) {
	for _, conflict := range []ListenerConflict{ListenerConflictHostname, ListenerConflictProtocol, ListenerConflictBindMode} {
		t.Run(string(conflict), func(t *testing.T) {
			winner := &GatewayListener{ParentInfo: ParentInfo{
				Port: 8080, Protocol: gwv1.HTTPProtocolType, Hostnames: []string{"example.com"},
			}}
			candidate := &ListenerSet{GatewayListener: *winner}
			switch conflict {
			case ListenerConflictProtocol:
				candidate.ParentInfo.Protocol = gwv1.HTTPSProtocolType
			case ListenerConflictBindMode:
				candidate.ParentInfo.Internal = true
			}
			listeners := []*GatewayListener{winner, &candidate.GatewayListener}
			validateListenerConflicts(listeners)
			assert.Same(t, winner, listeners[0])
			assert.NotSame(t, &candidate.GatewayListener, listeners[1])
			assert.Equal(t, conflict, listeners[1].Conflict)
			assert.Empty(t, candidate.Conflict)

			// Removing the winner lets the same collection-owned candidate recover.
			listeners = []*GatewayListener{&candidate.GatewayListener}
			validateListenerConflicts(listeners)
			assert.Same(t, &candidate.GatewayListener, listeners[0])
			assert.Empty(t, listeners[0].Conflict)
		})
	}
}

type fixedListenerParents []*ParentInfo

func (p fixedListenerParents) ParentsFor(krt.HandlerContext, utils.TypedNamespacedName) []*ParentInfo {
	return p
}

func TestArbitratedParentResolver(t *testing.T) {
	gw := types.NamespacedName{Namespace: "owner", Name: "gateway"}
	listener := &GatewayListener{
		Name: "tenant/set.http", ParentGateway: gw, Conflict: ListenerConflictHostname,
		ParentInfo: ParentInfo{ParentGateway: gw, ParentGatewayClassName: "agentgateway", ListenerKey: "tenant/set.http", Hostnames: []string{"tenant/*"}},
	}
	listeners := krt.NewStaticCollection(nil, []*GatewayListener{listener})
	stale := listener.ParentInfo
	stale.Hostnames = []string{"*/*"}
	stale.ParentGatewayClassName = "additional"
	wrongGateway := stale
	wrongGateway.ParentGateway.Name = "other"
	unknown := stale
	unknown.ListenerKey = "unknown"
	service := &ParentInfo{ServiceKey: &types.NamespacedName{Namespace: "tenant", Name: "service"}, ServicePorts: []gwv1.PortNumber{80}}
	listenerless := &ParentInfo{Hostnames: []string{"tenant/*"}}
	p := ArbitratedParentResolver{
		Listeners: listeners, Resolver: fixedListenerParents{nil, service, listenerless, &unknown, &wrongGateway, &stale, &stale},
		Gateways: krt.NewStaticCollection(nil, []*gwv1.Gateway{
			{Name: gw.Name, Namespace: gw.Namespace, Spec: gwv1.GatewaySpec{GatewayClassName: "agentgateway"}},
			{Name: "other", Namespace: gw.Namespace, Spec: gwv1.GatewaySpec{GatewayClassName: "agentgateway"}},
		}),
		AdditionalGatewayClasses: sets.New("additional"),
	}
	got := p.ParentsFor(krt.TestingDummyContext{}, utils.TypedNamespacedName{Kind: "ExtensionListenerSet"})
	assert.Equal(t, []*ParentInfo{service, listenerless, &listener.ParentInfo}, got)
	assert.Same(t, service, got[0])
	assert.Same(t, listenerless, got[1])
	assert.Equal(t, []string{"tenant/*"}, got[2].Hostnames)
}

func TestArbitratedParentResolverServiceMetadata(t *testing.T) {
	gw := types.NamespacedName{Namespace: "owner", Name: "gateway"}
	listener := &GatewayListener{
		Name: "owner/gateway.http", ParentGateway: gw,
		ParentInfo: ParentInfo{
			ParentGateway: gw, ListenerKey: "owner/gateway.http",
			Hostnames: []string{"owner/*"}, Port: 8080,
			AllowedKinds: []gwv1.RouteGroupKind{{Kind: "GRPCRoute"}},
		},
	}
	for _, key := range []string{"", "plugin/listener", listener.Name} {
		t.Run(key, func(t *testing.T) {
			service := &ParentInfo{
				ParentGateway: gw, ListenerKey: key,
				ServiceKey:   &types.NamespacedName{Namespace: "tenant", Name: "service"},
				ServicePorts: []gwv1.PortNumber{80, 443},
				Hostnames:    []string{"tenant/api.example.com"},
				AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}},
			}
			p := ArbitratedParentResolver{
				Listeners: krt.NewStaticCollection(nil, []*GatewayListener{listener}),
				Resolver:  fixedListenerParents{service},
			}
			got := p.ParentsFor(krt.TestingDummyContext{}, utils.TypedNamespacedName{Kind: "Service"})
			require.Len(t, got, 1)
			assert.Same(t, service, got[0])
			assert.Equal(t, &types.NamespacedName{Namespace: "tenant", Name: "service"}, got[0].ServiceKey)
			assert.Equal(t, []gwv1.PortNumber{80, 443}, got[0].ServicePorts)
			assert.Equal(t, []string{"tenant/api.example.com"}, got[0].Hostnames)
			assert.Equal(t, []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}}, got[0].AllowedKinds)
		})
	}
}

func TestArbitratedParentResolverDistinctServices(t *testing.T) {
	gw := types.NamespacedName{Namespace: "owner", Name: "gateway"}
	listener := &GatewayListener{
		Name: "owner/gateway.http", ParentGateway: gw,
		ParentInfo: ParentInfo{ParentGateway: gw, ListenerKey: "owner/gateway.http"},
	}
	first := &ParentInfo{
		ParentGateway: gw, ListenerKey: listener.Name,
		ServiceKey: &types.NamespacedName{Namespace: "tenant", Name: "first"}, ServicePorts: []gwv1.PortNumber{80},
	}
	second := *first
	second.ServiceKey = &types.NamespacedName{Namespace: "tenant", Name: "second"}
	p := ArbitratedParentResolver{
		Listeners: krt.NewStaticCollection(nil, []*GatewayListener{listener}),
		Resolver:  fixedListenerParents{first, &second},
	}
	got := p.ParentsFor(krt.TestingDummyContext{}, utils.TypedNamespacedName{Kind: "Service"})
	assert.Equal(t, []*ParentInfo{first, &second}, got)
}

func TestArbitratedParentResolverDelegatedMetadata(t *testing.T) {
	gw := types.NamespacedName{Namespace: "owner", Name: "gateway"}
	listener := &GatewayListener{
		Name: "owner/gateway.http", ParentGateway: gw,
		ParentInfo: ParentInfo{ParentGateway: gw, ListenerKey: "owner/gateway.http", Hostnames: []string{"owner/*"}},
	}
	for _, key := range []string{"plugin/listener", listener.Name} {
		t.Run(key, func(t *testing.T) {
			parent := &ParentInfo{
				ParentGateway: gw, ParentGatewayClassName: "agentgateway", ListenerKey: key,
				Hostnames: []string{"tenant/api.example.com"}, Port: 80,
				AllowedKinds: []gwv1.RouteGroupKind{{Kind: "HTTPRoute"}}, SectionName: "first",
			}
			second := *parent
			second.SectionName = "second"
			second.Hostnames = []string{"tenant/other.example.com"}
			p := ArbitratedParentResolver{
				Listeners: krt.NewStaticCollection(nil, []*GatewayListener{listener}),
				Gateways: krt.NewStaticCollection(nil, []*gwv1.Gateway{{
					Name: gw.Name, Namespace: gw.Namespace, Spec: gwv1.GatewaySpec{GatewayClassName: "additional"},
				}}),
				AdditionalGatewayClasses: sets.New("additional"),
				Resolver:                 fixedListenerParents{parent, &second},
			}
			got := p.ParentsFor(krt.TestingDummyContext{}, utils.TypedNamespacedName{Kind: "Gateway"})
			require.Len(t, got, 2)
			assert.Same(t, parent, got[0])
			assert.Same(t, &second, got[1])
		})
	}
}

func TestArbitratedParentResolverMissingGateway(t *testing.T) {
	gw := types.NamespacedName{Namespace: "owner", Name: "gateway"}
	listener := &GatewayListener{
		Name: "owner/gateway.http", ParentGateway: gw,
		ParentInfo: ParentInfo{ParentGateway: gw, ListenerKey: "owner/gateway.http"},
	}
	p := ArbitratedParentResolver{
		Listeners: krt.NewStaticCollection(nil, []*GatewayListener{listener}),
		Gateways:  krt.NewStaticCollection[*gwv1.Gateway](nil, nil),
		Resolver:  fixedListenerParents{&listener.ParentInfo},
	}
	assert.Empty(t, p.ParentsFor(krt.TestingDummyContext{}, utils.TypedNamespacedName{Kind: "Gateway"}))
}

func TestListenerConflictsUseServedHostname(t *testing.T) {
	for _, hostname := range []string{"", "example.com", "*.example.com"} {
		t.Run(hostname, func(t *testing.T) {
			winner := &GatewayListener{ParentInfo: ParentInfo{
				Port: 8080, Protocol: gwv1.HTTPProtocolType,
				OriginalHostname: hostname, Hostnames: []string{"*/" + hostname},
			}}
			loser := &GatewayListener{ParentInfo: ParentInfo{
				Port: 8080, Protocol: gwv1.HTTPProtocolType,
				OriginalHostname: hostname, Hostnames: []string{"tenant/" + hostname},
			}}
			listeners := []*GatewayListener{winner, loser}
			validateListenerConflicts(listeners)
			assert.Equal(t, ListenerConflict(ListenerConflictHostname), listeners[1].Conflict)
			assert.Empty(t, loser.Conflict)
		})
	}
}

func TestListenerDistinctHostnameSpecificity(t *testing.T) {
	var listeners []*GatewayListener
	for _, hostname := range []string{"", "*.example.com", "*.sub.example.com", "api.example.com", "other.example.com"} {
		listeners = append(listeners, &GatewayListener{ParentInfo: ParentInfo{
			Port: 8080, Protocol: gwv1.HTTPProtocolType, OriginalHostname: hostname,
			Hostnames: []string{"tenant/" + hostname},
		}})
	}
	validateListenerConflicts(listeners)
	for _, listener := range listeners {
		assert.Empty(t, listener.Conflict)
	}
}

func TestListenerConflictHTTPSAndTLS(t *testing.T) {
	cases := []struct {
		name     string
		first    gwv1.ProtocolType
		second   gwv1.ProtocolType
		hostname string
		want     ListenerConflict
	}{
		{name: "https then tls", first: gwv1.HTTPSProtocolType, second: gwv1.TLSProtocolType, hostname: "b.example.com"},
		{name: "tls then https", first: gwv1.TLSProtocolType, second: gwv1.HTTPSProtocolType, hostname: "b.example.com"},
		{name: "https then tls same hostname", first: gwv1.HTTPSProtocolType, second: gwv1.TLSProtocolType, hostname: "a.example.com", want: ListenerConflictHostname},
		{name: "https then http", first: gwv1.HTTPSProtocolType, second: gwv1.HTTPProtocolType, hostname: "b.example.com", want: ListenerConflictProtocol},
		{name: "tls then tcp", first: gwv1.TLSProtocolType, second: gwv1.TCPProtocolType, hostname: "b.example.com", want: ListenerConflictProtocol},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			listeners := []*GatewayListener{
				{ParentInfo: ParentInfo{Port: 443, Protocol: tc.first, Hostnames: []string{"default/a.example.com"}, OriginalHostname: "a.example.com"}},
				{ParentInfo: ParentInfo{Port: 443, Protocol: tc.second, Hostnames: []string{"default/" + tc.hostname}, OriginalHostname: tc.hostname}},
			}
			validateListenerConflicts(listeners)
			assert.Empty(t, listeners[0].Conflict)
			assert.Equal(t, tc.want, listeners[1].Conflict)
		})
	}
}

func TestListenerConflictSNIHostnameAcrossNamespaces(t *testing.T) {
	listener := func(ns string, protocol gwv1.ProtocolType, hostname string) *GatewayListener {
		key := hostname
		if key == "" {
			key = "*"
		}
		return &GatewayListener{ParentInfo: ParentInfo{
			Port: 443, Protocol: protocol, Hostnames: []string{ns + "/" + key}, OriginalHostname: hostname,
		}}
	}
	cases := []struct {
		name   string
		first  *GatewayListener
		second *GatewayListener
		want   ListenerConflict
	}{
		{
			name:   "https then tls",
			first:  listener("default", gwv1.HTTPSProtocolType, "foo.example.com"),
			second: listener("team1", gwv1.TLSProtocolType, "foo.example.com"),
			want:   ListenerConflictHostname,
		},
		{
			name:   "tls then https",
			first:  listener("default", gwv1.TLSProtocolType, "foo.example.com"),
			second: listener("team1", gwv1.HTTPSProtocolType, "foo.example.com"),
			want:   ListenerConflictHostname,
		},
		{
			name:   "https then https",
			first:  listener("default", gwv1.HTTPSProtocolType, "foo.example.com"),
			second: listener("team1", gwv1.HTTPSProtocolType, "foo.example.com"),
			want:   ListenerConflictHostname,
		},
		{
			name:   "no hostname",
			first:  listener("default", gwv1.HTTPSProtocolType, ""),
			second: listener("team1", gwv1.TLSProtocolType, ""),
			want:   ListenerConflictHostname,
		},
		{
			name:   "different hostname",
			first:  listener("default", gwv1.HTTPSProtocolType, "foo.example.com"),
			second: listener("team1", gwv1.TLSProtocolType, "bar.example.com"),
		},
		{
			name:   "http served hostname across namespaces",
			first:  listener("default", gwv1.HTTPProtocolType, "foo.example.com"),
			second: listener("team1", gwv1.HTTPProtocolType, "foo.example.com"),
			want:   ListenerConflictHostname,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			listeners := []*GatewayListener{tc.first, tc.second}
			validateListenerConflicts(listeners)
			assert.Empty(t, listeners[0].Conflict)
			assert.Equal(t, tc.want, listeners[1].Conflict)
		})
	}
}
