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

			// A fresh evaluation without the winner uses the unmodified candidate.
			listeners = []*GatewayListener{&candidate.GatewayListener}
			validateListenerConflicts(listeners)
			assert.Same(t, &candidate.GatewayListener, listeners[0])
			assert.Empty(t, listeners[0].Conflict)
		})
	}
}

func TestListenerDistinctHostnameSpecificity(t *testing.T) {
	for _, protocol := range []gwv1.ProtocolType{gwv1.HTTPProtocolType, gwv1.HTTPSProtocolType, gwv1.TLSProtocolType} {
		t.Run(string(protocol), func(t *testing.T) {
			var listeners []*GatewayListener
			for _, hostname := range []string{"", "*.example.com", "*.sub.example.com", "api.example.com", "other.example.com"} {
				listeners = append(listeners, &GatewayListener{ParentInfo: ParentInfo{
					Port: 8080, Protocol: protocol, OriginalHostname: hostname,
					Hostnames: []string{"tenant/" + hostname},
				}})
			}
			validateListenerConflicts(listeners)
			for _, listener := range listeners {
				assert.Empty(t, listener.Conflict)
			}
		})
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
	for _, protocols := range [][2]gwv1.ProtocolType{
		{gwv1.HTTPSProtocolType, gwv1.TLSProtocolType},
		{gwv1.TLSProtocolType, gwv1.HTTPSProtocolType},
		{gwv1.HTTPSProtocolType, gwv1.HTTPSProtocolType},
		{gwv1.HTTPProtocolType, gwv1.HTTPProtocolType},
	} {
		t.Run(string(protocols[0])+"/"+string(protocols[1]), func(t *testing.T) {
			for _, hostname := range []string{"", "example.com", "*.example.com"} {
				t.Run(hostname, func(t *testing.T) {
					key := hostname
					if key == "" {
						key = "*"
					}
					first := &GatewayListener{ParentInfo: ParentInfo{
						Port: 443, Protocol: protocols[0], OriginalHostname: hostname, Hostnames: []string{"*/" + key},
					}}
					second := &GatewayListener{ParentInfo: ParentInfo{
						Port: 443, Protocol: protocols[1], OriginalHostname: hostname, Hostnames: []string{"tenant/" + key},
					}}
					listeners := []*GatewayListener{first, second}
					validateListenerConflicts(listeners)
					assert.Empty(t, listeners[0].Conflict)
					assert.Equal(t, ListenerConflict(ListenerConflictHostname), listeners[1].Conflict)
					assert.Empty(t, second.Conflict)

					// Distinct served hostnames do not conflict even across namespaces.
					second.ParentInfo.OriginalHostname = "other.example.com"
					second.ParentInfo.Hostnames = []string{"tenant/other.example.com"}
					listeners = []*GatewayListener{first, second}
					validateListenerConflicts(listeners)
					assert.Empty(t, listeners[1].Conflict)
				})
			}
		})
	}
}
