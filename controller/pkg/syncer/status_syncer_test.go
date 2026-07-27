package syncer

import (
	"testing"

	"github.com/stretchr/testify/require"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"
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

func TestMergeXBackendAncestorStatuses_SortsOurEntriesOnly(t *testing.T) {
	our := "agentgateway.dev/agentgateway"
	other := "other.example/controller"
	existing := []gwxv1a1.BackendAncestorStatus{
		{ControllerName: gwv1.GatewayController(other), AncestorRef: gwv1.ParentReference{Name: "b"}},
		{ControllerName: gwv1.GatewayController(other), AncestorRef: gwv1.ParentReference{Name: "a"}},
	}
	desired := []gwxv1a1.BackendAncestorStatus{
		{ControllerName: gwv1.GatewayController(our), AncestorRef: gwv1.ParentReference{Name: "z"}},
		{ControllerName: gwv1.GatewayController(our), AncestorRef: gwv1.ParentReference{Name: "m"}},
	}

	out := mergeXBackendAncestorStatuses(our, existing, desired)
	require.Len(t, out, 4)
	require.Equal(t, "b", string(out[0].AncestorRef.Name))
	require.Equal(t, "a", string(out[1].AncestorRef.Name))
	require.Equal(t, "m", string(out[2].AncestorRef.Name))
	require.Equal(t, "z", string(out[3].AncestorRef.Name))
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

func TestMergeGatewayStatus_ArbitratesAcceptedInvalidParameters(t *testing.T) {
	tests := []struct {
		name                     string
		existing                 gwv1.GatewayStatus
		desired                  gwv1.GatewayStatus
		wantAcceptedStatus       metav1.ConditionStatus
		wantAcceptedReason       string
		wantProgrammedStatus     metav1.ConditionStatus
		wantProgrammedReason     string
		wantProgrammedGeneration int64
	}{
		{
			name: "desired default success does not overwrite live invalid parameters",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 1),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionTrue, gwv1.GatewayReasonAccepted, 1),
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 1),
			}},
			wantAcceptedStatus:       metav1.ConditionFalse,
			wantAcceptedReason:       string(gwv1.GatewayReasonInvalidParameters),
			wantProgrammedStatus:     metav1.ConditionFalse,
			wantProgrammedReason:     string(gwv1.GatewayReasonInvalid),
			wantProgrammedGeneration: 1,
		},
		{
			name: "newer desired default success clears stale invalid parameters",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 1),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionTrue, gwv1.GatewayReasonAccepted, 2),
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 2),
			}},
			wantAcceptedStatus:       metav1.ConditionTrue,
			wantAcceptedReason:       string(gwv1.GatewayReasonAccepted),
			wantProgrammedStatus:     metav1.ConditionTrue,
			wantProgrammedReason:     string(gwv1.GatewayReasonProgrammed),
			wantProgrammedGeneration: 2,
		},
		{
			name: "desired missing accepted does not clear live invalid parameters",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 2),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 2),
			}},
			wantAcceptedStatus:       metav1.ConditionFalse,
			wantAcceptedReason:       string(gwv1.GatewayReasonInvalidParameters),
			wantProgrammedStatus:     metav1.ConditionFalse,
			wantProgrammedReason:     string(gwv1.GatewayReasonInvalid),
			wantProgrammedGeneration: 2,
		},
		{
			name: "desired specific programmed negative is preserved with live invalid parameters",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 3),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionTrue, gwv1.GatewayReasonAccepted, 3),
				gatewayProgrammedCondition(metav1.ConditionFalse, gwv1.GatewayReasonAddressNotUsable, 3),
			}},
			wantAcceptedStatus:       metav1.ConditionFalse,
			wantAcceptedReason:       string(gwv1.GatewayReasonInvalidParameters),
			wantProgrammedStatus:     metav1.ConditionFalse,
			wantProgrammedReason:     string(gwv1.GatewayReasonAddressNotUsable),
			wantProgrammedGeneration: 3,
		},
		{
			name: "same generation invalid parameters does not overwrite live success",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionTrue, gwv1.GatewayReasonAccepted, 4),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 4),
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 4),
			}},
			wantAcceptedStatus:       metav1.ConditionTrue,
			wantAcceptedReason:       string(gwv1.GatewayReasonAccepted),
			wantProgrammedStatus:     metav1.ConditionTrue,
			wantProgrammedReason:     string(gwv1.GatewayReasonProgrammed),
			wantProgrammedGeneration: 4,
		},
		{
			name: "newer invalid parameters overwrites live success",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionTrue, gwv1.GatewayReasonAccepted, 5),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 6),
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 6),
			}},
			wantAcceptedStatus:       metav1.ConditionFalse,
			wantAcceptedReason:       string(gwv1.GatewayReasonInvalidParameters),
			wantProgrammedStatus:     metav1.ConditionTrue,
			wantProgrammedReason:     string(gwv1.GatewayReasonProgrammed),
			wantProgrammedGeneration: 6,
		},
		{
			name: "same generation invalid parameters overwrites unsupported address",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonUnsupportedAddress, 7),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 7),
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 7),
			}},
			wantAcceptedStatus:       metav1.ConditionFalse,
			wantAcceptedReason:       string(gwv1.GatewayReasonInvalidParameters),
			wantProgrammedStatus:     metav1.ConditionTrue,
			wantProgrammedReason:     string(gwv1.GatewayReasonProgrammed),
			wantProgrammedGeneration: 7,
		},
		{
			name: "same generation unsupported address does not overwrite live invalid parameters",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 8),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonUnsupportedAddress, 8),
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 8),
			}},
			wantAcceptedStatus:       metav1.ConditionFalse,
			wantAcceptedReason:       string(gwv1.GatewayReasonInvalidParameters),
			wantProgrammedStatus:     metav1.ConditionFalse,
			wantProgrammedReason:     string(gwv1.GatewayReasonInvalid),
			wantProgrammedGeneration: 8,
		},
		{
			name: "newer unsupported address overwrites live invalid parameters",
			existing: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonInvalidParameters, 9),
			}},
			desired: gwv1.GatewayStatus{Conditions: []metav1.Condition{
				gatewayAcceptedCondition(metav1.ConditionFalse, gwv1.GatewayReasonUnsupportedAddress, 10),
				gatewayProgrammedCondition(metav1.ConditionTrue, gwv1.GatewayReasonProgrammed, 10),
			}},
			wantAcceptedStatus:       metav1.ConditionFalse,
			wantAcceptedReason:       string(gwv1.GatewayReasonUnsupportedAddress),
			wantProgrammedStatus:     metav1.ConditionTrue,
			wantProgrammedReason:     string(gwv1.GatewayReasonProgrammed),
			wantProgrammedGeneration: 10,
		},
	}

	for _, testCase := range tests {
		t.Run(testCase.name, func(t *testing.T) {
			merged := mergeGatewayStatus(testCase.existing, testCase.desired)
			accepted := meta.FindStatusCondition(merged.Conditions, string(gwv1.GatewayConditionAccepted))
			require.NotNil(t, accepted)
			require.Equal(t, testCase.wantAcceptedStatus, accepted.Status)
			require.Equal(t, testCase.wantAcceptedReason, accepted.Reason)

			programmed := meta.FindStatusCondition(merged.Conditions, string(gwv1.GatewayConditionProgrammed))
			require.NotNil(t, programmed)
			require.Equal(t, testCase.wantProgrammedStatus, programmed.Status)
			require.Equal(t, testCase.wantProgrammedReason, programmed.Reason)
			require.Equal(t, testCase.wantProgrammedGeneration, programmed.ObservedGeneration)
		})
	}
}

func gatewayAcceptedCondition(status metav1.ConditionStatus, reason gwv1.GatewayConditionReason, generation int64) metav1.Condition {
	return metav1.Condition{
		Type:               string(gwv1.GatewayConditionAccepted),
		Status:             status,
		ObservedGeneration: generation,
		Reason:             string(reason),
		Message:            "accepted message",
	}
}

func gatewayProgrammedCondition(status metav1.ConditionStatus, reason gwv1.GatewayConditionReason, generation int64) metav1.Condition {
	return metav1.Condition{
		Type:               string(gwv1.GatewayConditionProgrammed),
		Status:             status,
		ObservedGeneration: generation,
		Reason:             string(reason),
		Message:            "programmed message",
	}
}
