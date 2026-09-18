package translator

import (
	"testing"

	"github.com/stretchr/testify/assert"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
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
