package agentgatewaysyncer

import (
	"testing"

	"github.com/stretchr/testify/require"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

func TestMergePolicyAncestorStatuses_SortsOurEntriesOnly(t *testing.T) {
	our := "agentgateway.dev/agentgateway"
	other := "kgateway.dev/kgateway"

	existing := []gwv1.PolicyAncestorStatus{
		{ControllerName: gwv1.GatewayController(other), AncestorRef: gwv1.ParentReference{Name: "b"}},
		{ControllerName: gwv1.GatewayController(other), AncestorRef: gwv1.ParentReference{Name: "a"}},
	}
	desired := []gwv1.PolicyAncestorStatus{
		{ControllerName: gwv1.GatewayController(our), AncestorRef: gwv1.ParentReference{Name: "z"}},
		{ControllerName: gwv1.GatewayController(our), AncestorRef: gwv1.ParentReference{Name: "m"}},
	}

	out := mergePolicyAncestorStatuses(our, existing, desired)
	require.Len(t, out, 4)

	// Other-controller entries preserved (including order).
	require.Equal(t, string(out[0].ControllerName), other)
	require.Equal(t, string(out[1].ControllerName), other)
	require.Equal(t, string(out[0].AncestorRef.Name), "b")
	require.Equal(t, string(out[1].AncestorRef.Name), "a")

	// Our entries appended, but sorted deterministically.
	require.Equal(t, string(out[2].ControllerName), our)
	require.Equal(t, string(out[3].ControllerName), our)
	require.Equal(t, string(out[2].AncestorRef.Name), "m")
	require.Equal(t, string(out[3].AncestorRef.Name), "z")
}

func TestMergeRouteParentStatuses_SortsOurEntriesOnly(t *testing.T) {
	our := "agentgateway.dev/agentgateway"
	other := "kgateway.dev/kgateway"

	existing := []gwv1.RouteParentStatus{
		{ControllerName: gwv1.GatewayController(other), ParentRef: gwv1.ParentReference{Name: "b"}},
		{ControllerName: gwv1.GatewayController(other), ParentRef: gwv1.ParentReference{Name: "a"}},
	}
	desired := []gwv1.RouteParentStatus{
		{ControllerName: gwv1.GatewayController(our), ParentRef: gwv1.ParentReference{Name: "z"}},
		{ControllerName: gwv1.GatewayController(our), ParentRef: gwv1.ParentReference{Name: "m"}},
	}

	out := mergeRouteParentStatuses(our, existing, desired)
	require.Len(t, out, 4)

	// Other-controller entries preserved (including order).
	require.Equal(t, string(out[0].ControllerName), other)
	require.Equal(t, string(out[1].ControllerName), other)
	require.Equal(t, string(out[0].ParentRef.Name), "b")
	require.Equal(t, string(out[1].ParentRef.Name), "a")

	// Our entries appended, but sorted deterministically.
	require.Equal(t, string(out[2].ControllerName), our)
	require.Equal(t, string(out[3].ControllerName), our)
	require.Equal(t, string(out[2].ParentRef.Name), "m")
	require.Equal(t, string(out[3].ParentRef.Name), "z")
}

func TestMergeGatewayAddresses_SortsOutput(t *testing.T) {
	// When desired is empty, we keep existing but still sort it for stability.
	existing := []gwv1.GatewayStatusAddress{
		{Value: "2.2.2.2"}, // Type nil => IPAddress
		{Value: "1.1.1.1"},
	}
	out := mergeGatewayAddresses(existing, nil)
	require.Len(t, out, 2)
	require.Equal(t, "1.1.1.1", out[0].Value)
	require.Equal(t, "2.2.2.2", out[1].Value)

	// When desired is non-empty, it wins, but is sorted.
	hostname := gwv1.AddressType("Hostname")
	desired := []gwv1.GatewayStatusAddress{
		{Type: &hostname, Value: "b.example.com"},
		{Type: &hostname, Value: "a.example.com"},
	}
	out2 := mergeGatewayAddresses(existing, desired)
	require.Len(t, out2, 2)
	require.Equal(t, "a.example.com", out2[0].Value)
	require.Equal(t, "b.example.com", out2[1].Value)
}
