package translator

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
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
		ParentInfo: ParentInfo{ParentGateway: gw, ListenerKey: "tenant/set.http", Hostnames: []string{"tenant/*"}},
	}
	listeners := krt.NewStaticCollection(nil, []*GatewayListener{listener})
	stale := listener.ParentInfo
	stale.Hostnames = []string{"*/*"}
	wrongGateway := stale
	wrongGateway.ParentGateway.Name = "other"
	unknown := stale
	unknown.ListenerKey = "unknown"
	p := ArbitratedParentResolver{Listeners: listeners, Resolver: fixedListenerParents{nil, &unknown, &wrongGateway, &stale, &stale}}
	got := p.ParentsFor(krt.TestingDummyContext{}, utils.TypedNamespacedName{Kind: "ExtensionListenerSet"})
	assert.Equal(t, []*ParentInfo{&listener.ParentInfo}, got)
	assert.Equal(t, []string{"tenant/*"}, got[0].Hostnames)
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
