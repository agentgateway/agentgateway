package remotehttp

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/ptr"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
)

func TestTargetSectionMatcher(t *testing.T) {
	matcher := newTargetSectionMatcher([]string{"8443", "https"})

	require.Equal(t, sectionWholeResourceMatch, matcher.Match(nil))
	require.Equal(t, sectionExactMatch, matcher.Match(ptr.Of(gwv1.SectionName("https"))))
	require.Equal(t, sectionNoMatch, matcher.Match(ptr.Of(gwv1.SectionName("http"))))
}

func TestBestMatchingAgentgatewayPolicy(t *testing.T) {
	older := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:              "older-whole",
			Namespace:         "default",
			CreationTimestamp: metav1.Unix(10, 0),
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
					Group: gwv1.Group(""),
					Kind:  gwv1.Kind("Service"),
					Name:  gwv1.ObjectName("oauth2"),
				},
			}},
		},
	}
	newerExact := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:              "newer-exact",
			Namespace:         "default",
			CreationTimestamp: metav1.Unix(20, 0),
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
					Group: gwv1.Group(""),
					Kind:  gwv1.Kind("Service"),
					Name:  gwv1.ObjectName("oauth2"),
				},
				SectionName: ptr.Of(gwv1.SectionName("https")),
			}},
		},
	}
	matcher := newTargetSectionMatcher([]string{"https"})

	selected := bestMatchingAgentgatewayPolicy(
		[]*agentgateway.AgentgatewayPolicy{older, newerExact},
		"",
		"Service",
		"oauth2",
		matcher,
	)
	require.Same(t, newerExact, selected)

	olderExact := older.DeepCopy()
	olderExact.Name = "older-exact"
	olderExact.Spec.TargetRefs[0].SectionName = ptr.Of(gwv1.SectionName("https"))
	selected = bestMatchingAgentgatewayPolicy(
		[]*agentgateway.AgentgatewayPolicy{olderExact, newerExact},
		"",
		"Service",
		"oauth2",
		matcher,
	)
	require.Same(t, olderExact, selected)
}

func TestBestMatchingBackendTLSPolicyAndEquality(t *testing.T) {
	whole := &gwv1.BackendTLSPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:              "whole",
			Namespace:         "default",
			CreationTimestamp: metav1.Unix(10, 0),
		},
		Spec: gwv1.BackendTLSPolicySpec{
			TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
					Group: gwv1.Group(""),
					Kind:  gwv1.Kind("Service"),
					Name:  gwv1.ObjectName("oauth2"),
				},
			}},
		},
	}
	exact := &gwv1.BackendTLSPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:              "exact",
			Namespace:         "default",
			CreationTimestamp: metav1.Unix(20, 0),
		},
		Spec: gwv1.BackendTLSPolicySpec{
			TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
					Group: gwv1.Group(""),
					Kind:  gwv1.Kind("Service"),
					Name:  gwv1.ObjectName("oauth2"),
				},
				SectionName: ptr.Of(gwv1.SectionName("https")),
			}},
		},
	}
	matcher := newTargetSectionMatcher([]string{"https"})

	selected := bestMatchingBackendTLSPolicy(
		[]*gwv1.BackendTLSPolicy{whole, exact},
		"",
		"Service",
		"oauth2",
		matcher,
	)
	require.Same(t, exact, selected)
	require.True(t, backendTLSTargetEqual(exact.Spec.TargetRefs[0], exact.Spec.TargetRefs[0]))
	require.False(t, backendTLSTargetEqual(whole.Spec.TargetRefs[0], exact.Spec.TargetRefs[0]))

	sameValueDifferentPointer := exact.DeepCopy()
	sameValueDifferentPointer.Spec.TargetRefs[0].SectionName = ptr.Of(gwv1.SectionName("https"))
	require.True(t, backendTLSTargetEqual(exact.Spec.TargetRefs[0], sameValueDifferentPointer.Spec.TargetRefs[0]))
}
