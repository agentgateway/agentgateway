package syncer

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/config/protocol"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	agwir "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
)

// gwListener builds a minimal GatewayListener for buildBindsFromGateway tests.
// buildBindsFromGateway only reads ParentGateway, ParentInfo.Port, ParentInfo.Protocol
// and Conflict, so the other fields are intentionally left unset.
func gwListener(ns, name string, port int, proto gwv1.ProtocolType, hostname string, conflict translator.ListenerConflict) *translator.GatewayListener {
	return &translator.GatewayListener{
		Name:          name,
		ParentGateway: types.NamespacedName{Namespace: ns, Name: name},
		ParentInfo: translator.ParentInfo{
			Port:             gwv1.PortNumber(port),
			Protocol:         proto,
			OriginalHostname: hostname,
		},
		Conflict: conflict,
		Valid:    conflict == "",
	}
}

// bindsByKey extracts the binds from the produced resources, keyed by Bind.Key.
func bindsByKey(t *testing.T, resources []agwir.AgwResource) map[string]*api.Bind {
	t.Helper()
	out := make(map[string]*api.Bind, len(resources))
	for _, r := range resources {
		b := r.Resource.GetBind()
		require.NotNil(t, b, "resource is not a Bind")
		out[b.Key] = b
	}
	return out
}

func TestBuildBindsFromGateway_Empty(t *testing.T) {
	s := &Syncer{}
	require.Nil(t, s.buildBindsFromGateway(nil))
	require.Nil(t, s.buildBindsFromGateway([]*translator.GatewayListener{}))
}

func TestBuildBindsFromGateway_SingleListenerProtocols(t *testing.T) {
	cases := []struct {
		name         string
		proto        gwv1.ProtocolType
		wantProtocol api.Bind_Protocol
		wantTunnel   api.Bind_TunnelProtocol
	}{
		{"http", gwv1.HTTPProtocolType, api.Bind_HTTP, api.Bind_DIRECT},
		{"https", gwv1.HTTPSProtocolType, api.Bind_TLS, api.Bind_DIRECT},
		{"tls", gwv1.TLSProtocolType, api.Bind_TLS, api.Bind_DIRECT},
		{"tcp", gwv1.TCPProtocolType, api.Bind_TCP, api.Bind_DIRECT},
		// HBONE binds use HTTP as a placeholder protocol but set the tunnel to HBONE_GATEWAY.
		{"hbone", gwv1.ProtocolType(protocol.HBONE), api.Bind_HTTP, api.Bind_HBONE_GATEWAY},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := &Syncer{}
			listeners := []*translator.GatewayListener{
				gwListener("default", "gw", 8080, tc.proto, "", ""),
			}
			binds := bindsByKey(t, s.buildBindsFromGateway(listeners))
			require.Len(t, binds, 1)
			b := binds["8080/default/gw"]
			require.NotNil(t, b)
			require.Equal(t, uint32(8080), b.Port)
			require.Equal(t, tc.wantProtocol, b.Protocol)
			require.Equal(t, tc.wantTunnel, b.TunnelProtocol)
		})
	}
}

func TestBuildBindsFromGateway_KeyFormatMatchesListenerBindKey(t *testing.T) {
	// The bind Key must match the BindKey the listener resources reference:
	// "port/namespace/name".
	s := &Syncer{}
	listeners := []*translator.GatewayListener{
		gwListener("ns1", "my-gw", 443, gwv1.HTTPSProtocolType, "example.com", ""),
	}
	resources := s.buildBindsFromGateway(listeners)
	require.Len(t, resources, 1)

	binds := bindsByKey(t, resources)
	b := binds["443/ns1/my-gw"]
	require.NotNil(t, b, "bind key should be port/namespace/name")
	// The owning gateway is propagated onto the resource.
	require.Equal(t, types.NamespacedName{Namespace: "ns1", Name: "my-gw"}, resources[0].Gateway)
}

func TestBuildBindsFromGateway_SamePortSameProtocolDifferentHostnames(t *testing.T) {
	// Multiple valid (non-conflicting) listeners can share a port when their
	// hostnames are disjoint; they are guaranteed to share the same protocol.
	// They must collapse into a single bind for that port.
	s := &Syncer{}
	listeners := []*translator.GatewayListener{
		gwListener("default", "gw", 8080, gwv1.HTTPProtocolType, "a.example.com", ""),
		gwListener("default", "gw", 8080, gwv1.HTTPProtocolType, "b.example.com", ""),
	}
	binds := bindsByKey(t, s.buildBindsFromGateway(listeners))
	require.Len(t, binds, 1)
	b := binds["8080/default/gw"]
	require.NotNil(t, b)
	require.Equal(t, api.Bind_HTTP, b.Protocol)
}

func TestBuildBindsFromGateway_DistinctPorts(t *testing.T) {
	// One bind per distinct port, emitted in ascending port order for determinism.
	s := &Syncer{}
	listeners := []*translator.GatewayListener{
		gwListener("default", "gw", 8080, gwv1.HTTPProtocolType, "", ""),
		gwListener("default", "gw", 80, gwv1.HTTPProtocolType, "", ""),
		gwListener("default", "gw", 443, gwv1.HTTPSProtocolType, "", ""),
	}
	resources := s.buildBindsFromGateway(listeners)
	require.Len(t, resources, 3)

	// Deterministic ascending-by-port ordering.
	gotPorts := []uint32{
		resources[0].Resource.GetBind().Port,
		resources[1].Resource.GetBind().Port,
		resources[2].Resource.GetBind().Port,
	}
	require.Equal(t, []uint32{80, 443, 8080}, gotPorts)

	binds := bindsByKey(t, resources)
	require.Equal(t, api.Bind_HTTP, binds["80/default/gw"].Protocol)
	require.Equal(t, api.Bind_TLS, binds["443/default/gw"].Protocol)
	require.Equal(t, api.Bind_HTTP, binds["8080/default/gw"].Protocol)
}

func TestBuildBindsFromGateway_ConflictedListenerIgnored(t *testing.T) {
	// The first listener on a port is always the non-conflicting winner.
	// A later conflicting listener on the same port (e.g. a protocol conflict)
	// must not influence the bind protocol.
	s := &Syncer{}
	listeners := []*translator.GatewayListener{
		gwListener("default", "gw", 8080, gwv1.HTTPProtocolType, "", ""),
		gwListener("default", "gw", 8080, gwv1.HTTPSProtocolType, "", translator.ListenerConflictProtocol),
	}
	binds := bindsByKey(t, s.buildBindsFromGateway(listeners))
	require.Len(t, binds, 1)
	b := binds["8080/default/gw"]
	require.NotNil(t, b)
	// Winner is the non-conflicting HTTP listener, not the conflicting HTTPS one.
	require.Equal(t, api.Bind_HTTP, b.Protocol)
}

func TestBuildBindsFromGateway_HostnameConflictIgnored(t *testing.T) {
	// A hostname-conflicting listener also must not contribute to the bind.
	s := &Syncer{}
	listeners := []*translator.GatewayListener{
		gwListener("default", "gw", 8080, gwv1.HTTPProtocolType, "a.example.com", ""),
		gwListener("default", "gw", 8080, gwv1.HTTPProtocolType, "a.example.com", translator.ListenerConflictHostname),
	}
	binds := bindsByKey(t, s.buildBindsFromGateway(listeners))
	require.Len(t, binds, 1)
	require.Equal(t, api.Bind_HTTP, binds["8080/default/gw"].Protocol)
}

func TestBuildBindsFromGateway_AllConflictedFallsBackToDefault(t *testing.T) {
	// Defensive: if every listener on a port is conflicted (not expected from the
	// conflict pass, where the first listener always wins), the bind falls back to
	// the zero-value protocol/tunnel (Bind_HTTP / Bind_DIRECT) rather than being
	// dropped, so the port still has a bind.
	s := &Syncer{}
	listeners := []*translator.GatewayListener{
		gwListener("default", "gw", 8080, gwv1.HTTPSProtocolType, "", translator.ListenerConflictProtocol),
	}
	binds := bindsByKey(t, s.buildBindsFromGateway(listeners))
	require.Len(t, binds, 1)
	b := binds["8080/default/gw"]
	require.NotNil(t, b)
	require.Equal(t, api.Bind_HTTP, b.Protocol)
	require.Equal(t, api.Bind_DIRECT, b.TunnelProtocol)
}

func TestBuildBindsFromGateway_HBONETunnelPreservedAgainstConflictingListener(t *testing.T) {
	// An HBONE listener wins port 15008 and sets the tunnel to HBONE_GATEWAY.
	// A second, non-HBONE listener on the same port necessarily has a different
	// protocol, so the conflict pass marks it ListenerConflictProtocol. That
	// conflicting listener must not contribute to (or reset) the bind, so the
	// tunnel stays HBONE_GATEWAY.
	s := &Syncer{}
	listeners := []*translator.GatewayListener{
		gwListener("default", "gw", 15008, gwv1.ProtocolType(protocol.HBONE), "", ""),
		gwListener("default", "gw", 15008, gwv1.HTTPProtocolType, "", translator.ListenerConflictProtocol),
	}
	binds := bindsByKey(t, s.buildBindsFromGateway(listeners))
	require.Len(t, binds, 1)
	b := binds["15008/default/gw"]
	require.NotNil(t, b)
	require.Equal(t, api.Bind_HTTP, b.Protocol)
	require.Equal(t, api.Bind_HBONE_GATEWAY, b.TunnelProtocol)
}
