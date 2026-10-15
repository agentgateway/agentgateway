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
			name:   "http unchanged",
			first:  listener("default", gwv1.HTTPProtocolType, "foo.example.com"),
			second: listener("team1", gwv1.HTTPProtocolType, "foo.example.com"),
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
