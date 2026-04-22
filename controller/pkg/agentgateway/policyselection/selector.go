package policyselection

import (
	"cmp"
	"fmt"
	"slices"

	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	krtpkg "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
)

type Selector interface {
	BestMatchingAgentgatewayPolicy(
		krtctx krt.HandlerContext,
		namespace, group, kind, name string,
		exactSections []string,
	) *agentgateway.AgentgatewayPolicy
	BestMatchingBackendTLSPolicy(
		krtctx krt.HandlerContext,
		namespace, group, kind, name string,
		exactSections []string,
	) *gwv1.BackendTLSPolicy
}

type policyTargetRefKey struct {
	Group     string
	Kind      string
	Name      string
	Namespace string
}

type backendTLSPolicyTargetRefKey struct {
	Group     string
	Name      string
	Kind      string
	Namespace string
}

func (k policyTargetRefKey) String() string {
	return fmt.Sprintf("%s:%s:%s:%s", k.Group, k.Kind, k.Namespace, k.Name)
}

func (k backendTLSPolicyTargetRefKey) String() string {
	return fmt.Sprintf("%s:%s:%s:%s", k.Group, k.Namespace, k.Kind, k.Name)
}

type sectionMatchRank uint8

const (
	sectionNoMatch sectionMatchRank = iota
	sectionWholeResourceMatch
	sectionExactMatch
)

type sectionMatcher struct {
	exact []string
}

type defaultSelector struct {
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	backendTLSPolicies   krt.Collection[*gwv1.BackendTLSPolicy]
	policiesByTargetRef  krt.Index[policyTargetRefKey, *agentgateway.AgentgatewayPolicy]
	backendTLSByTarget   krt.Index[backendTLSPolicyTargetRefKey, *gwv1.BackendTLSPolicy]
}

func NewSelector(
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy],
	backendTLSPolicies krt.Collection[*gwv1.BackendTLSPolicy],
) Selector {
	return &defaultSelector{
		agentgatewayPolicies: agentgatewayPolicies,
		backendTLSPolicies:   backendTLSPolicies,
		policiesByTargetRef:  newPolicyTargetRefIndex(agentgatewayPolicies),
		backendTLSByTarget:   newBackendTLSPolicyTargetRefIndex(backendTLSPolicies),
	}
}

func HasHigherPriority(a, b metav1.Object) bool {
	ts := a.GetCreationTimestamp().Compare(b.GetCreationTimestamp().Time)
	if ts < 0 {
		return true
	}
	if ts > 0 {
		return false
	}
	ns := cmp.Compare(a.GetNamespace(), b.GetNamespace())
	if ns < 0 {
		return true
	}
	if ns > 0 {
		return false
	}
	return a.GetName() < b.GetName()
}

func (s *defaultSelector) BestMatchingAgentgatewayPolicy(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	exactSections []string,
) *agentgateway.AgentgatewayPolicy {
	candidates := krt.Fetch(
		krtctx,
		s.agentgatewayPolicies,
		krt.FilterIndex(s.policiesByTargetRef, policyTargetRefKey{
			Name:      name,
			Kind:      kind,
			Group:     group,
			Namespace: namespace,
		}),
	)
	return bestMatchingAgentgatewayPolicy(candidates, group, kind, name, newSectionMatcher(exactSections))
}

func (s *defaultSelector) BestMatchingBackendTLSPolicy(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	exactSections []string,
) *gwv1.BackendTLSPolicy {
	candidates := krt.Fetch(
		krtctx,
		s.backendTLSPolicies,
		krt.FilterIndex(s.backendTLSByTarget, backendTLSPolicyTargetRefKey{
			Group:     group,
			Name:      name,
			Kind:      kind,
			Namespace: namespace,
		}),
	)
	return bestMatchingBackendTLSPolicy(candidates, group, kind, name, newSectionMatcher(exactSections))
}

func newPolicyTargetRefIndex(
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy],
) krt.Index[policyTargetRefKey, *agentgateway.AgentgatewayPolicy] {
	return krtpkg.UnnamedIndex(agentgatewayPolicies, func(in *agentgateway.AgentgatewayPolicy) []policyTargetRefKey {
		keys := make([]policyTargetRefKey, 0, len(in.Spec.TargetRefs))
		for _, ref := range in.Spec.TargetRefs {
			keys = append(keys, policyTargetRefKey{
				Name:      string(ref.Name),
				Kind:      string(ref.Kind),
				Group:     string(ref.Group),
				Namespace: in.Namespace,
			})
		}
		return keys
	})
}

func newBackendTLSPolicyTargetRefIndex(
	backendTLSPolicies krt.Collection[*gwv1.BackendTLSPolicy],
) krt.Index[backendTLSPolicyTargetRefKey, *gwv1.BackendTLSPolicy] {
	return krtpkg.UnnamedIndex(backendTLSPolicies, func(in *gwv1.BackendTLSPolicy) []backendTLSPolicyTargetRefKey {
		keys := make([]backendTLSPolicyTargetRefKey, 0, len(in.Spec.TargetRefs))
		for _, ref := range in.Spec.TargetRefs {
			keys = append(keys, backendTLSPolicyTargetRefKey{
				Group:     string(ref.Group),
				Name:      string(ref.Name),
				Kind:      string(ref.Kind),
				Namespace: in.Namespace,
			})
		}
		return keys
	})
}

func newSectionMatcher(exact []string) sectionMatcher {
	return sectionMatcher{exact: exact}
}

func (m sectionMatcher) Match(sectionName *gwv1.SectionName) sectionMatchRank {
	if sectionName == nil {
		return sectionWholeResourceMatch
	}
	if slices.Contains(m.exact, string(*sectionName)) {
		return sectionExactMatch
	}
	return sectionNoMatch
}

func bestMatchingAgentgatewayPolicy(
	candidates []*agentgateway.AgentgatewayPolicy,
	group, kind, name string,
	matcher sectionMatcher,
) *agentgateway.AgentgatewayPolicy {
	var (
		selected *agentgateway.AgentgatewayPolicy
		bestRank sectionMatchRank
	)
	for _, candidate := range candidates {
		rank := bestMatchingPolicyTargetRank(candidate.Spec.TargetRefs, group, kind, name, matcher)
		if rank == sectionNoMatch {
			continue
		}
		if selected == nil || rank > bestRank || (rank == bestRank && HasHigherPriority(candidate, selected)) {
			selected = candidate
			bestRank = rank
		}
	}
	return selected
}

func bestMatchingBackendTLSPolicy(
	candidates []*gwv1.BackendTLSPolicy,
	group, kind, name string,
	matcher sectionMatcher,
) *gwv1.BackendTLSPolicy {
	var (
		selected *gwv1.BackendTLSPolicy
		bestRank sectionMatchRank
	)
	for _, candidate := range candidates {
		rank := bestMatchingBackendTLSTargetRank(candidate.Spec.TargetRefs, group, kind, name, matcher)
		if rank == sectionNoMatch {
			continue
		}
		if selected == nil || rank > bestRank || (rank == bestRank && HasHigherPriority(candidate, selected)) {
			selected = candidate
			bestRank = rank
		}
	}
	return selected
}

func bestMatchingPolicyTargetRank(
	targetRefs []shared.LocalPolicyTargetReferenceWithSectionName,
	group, kind, name string,
	matcher sectionMatcher,
) sectionMatchRank {
	best := sectionNoMatch
	for _, targetRef := range targetRefs {
		if string(targetRef.Group) != group || string(targetRef.Kind) != kind || string(targetRef.Name) != name {
			continue
		}
		if rank := matcher.Match(targetRef.SectionName); rank > best {
			best = rank
		}
	}
	return best
}

func bestMatchingBackendTLSTargetRank(
	targetRefs []gwv1.LocalPolicyTargetReferenceWithSectionName,
	group, kind, name string,
	matcher sectionMatcher,
) sectionMatchRank {
	best := sectionNoMatch
	for _, targetRef := range targetRefs {
		if string(targetRef.Group) != group || string(targetRef.Kind) != kind || string(targetRef.Name) != name {
			continue
		}
		if rank := matcher.Match(targetRef.SectionName); rank > best {
			best = rank
		}
	}
	return best
}
