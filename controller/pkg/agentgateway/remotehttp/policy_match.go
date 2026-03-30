package remotehttp

import (
	"fmt"
	"slices"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

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

type targetSectionMatcher struct {
	exact []string
}

func newTargetSectionMatcher(exact []string) targetSectionMatcher {
	return targetSectionMatcher{exact: exact}
}

func (m targetSectionMatcher) Match(sectionName *gwv1.SectionName) sectionMatchRank {
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
	matcher targetSectionMatcher,
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
		if selected == nil || rank > bestRank || (rank == bestRank && utils.HigherPriorityPolicy(candidate, selected)) {
			selected = candidate
			bestRank = rank
		}
	}
	return selected
}

func (r *defaultResolver) fetchBestMatchingAgentgatewayPolicy(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	matcher targetSectionMatcher,
) *agentgateway.AgentgatewayPolicy {
	candidates := krt.Fetch(
		krtctx,
		r.agentgatewayPolicies,
		krt.FilterIndex(r.policiesByTargetRef, policyTargetRefKey{
			Name:      name,
			Kind:      kind,
			Group:     group,
			Namespace: namespace,
		}),
	)
	return bestMatchingAgentgatewayPolicy(candidates, group, kind, name, matcher)
}

func bestMatchingBackendTLSPolicy(
	candidates []*gwv1.BackendTLSPolicy,
	group, kind, name string,
	matcher targetSectionMatcher,
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
		if selected == nil || rank > bestRank || (rank == bestRank && utils.HigherPriorityPolicy(candidate, selected)) {
			selected = candidate
			bestRank = rank
		}
	}
	return selected
}

func (r *defaultResolver) fetchBestMatchingBackendTLSPolicy(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	matcher targetSectionMatcher,
) *gwv1.BackendTLSPolicy {
	candidates := krt.Fetch(
		krtctx,
		r.backendTLSPolicies,
		krt.FilterIndex(r.backendTLSByTarget, backendTLSPolicyTargetRefKey{
			Group:     group,
			Name:      name,
			Kind:      kind,
			Namespace: namespace,
		}),
	)
	return bestMatchingBackendTLSPolicy(candidates, group, kind, name, matcher)
}

func backendTLSTargetEqual(a, b gwv1.LocalPolicyTargetReferenceWithSectionName) bool {
	return a.Group == b.Group &&
		a.Kind == b.Kind &&
		a.Name == b.Name &&
		ptr.Equal(a.SectionName, b.SectionName)
}

func bestMatchingPolicyTargetRank(
	targetRefs []shared.LocalPolicyTargetReferenceWithSectionName,
	group, kind, name string,
	matcher targetSectionMatcher,
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
	matcher targetSectionMatcher,
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
