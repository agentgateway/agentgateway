package plugins

import (
	"sort"

	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

// BuildPolicyAncestorBackendsCollection records backend references used by policies whose
// runtime execution happens inside the gateway, so target-scoped backend policies such as
// BackendTLSPolicy can still be projected to the owning gateways.
func BuildPolicyAncestorBackendsCollection(agw *AgwCollections) krt.Collection[*utils.AncestorBackend] {
	return krt.NewManyCollection(
		agw.AgentgatewayPolicies,
		func(ctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) []*utils.AncestorBackend {
			return collectPolicyAncestorBackends(ctx, policy, agw)
		},
		agw.KrtOpts.ToOptions("PolicyAncestorBackends")...,
	)
}

func collectPolicyAncestorBackends(
	ctx krt.HandlerContext,
	policy *agentgateway.AgentgatewayPolicy,
	agw *AgwCollections,
) []*utils.AncestorBackend {
	if policy.Spec.Traffic == nil || policy.Spec.Traffic.OAuth2 == nil || policy.Spec.Traffic.OAuth2.BackendRef == nil {
		return nil
	}

	backendRef := policyBackendTargetRef(policy.Namespace, *policy.Spec.Traffic.OAuth2.BackendRef)
	if backendRef == nil {
		return nil
	}

	gateways := make(map[types.NamespacedName]struct{})
	for _, target := range collectPolicyAttachmentTargets(ctx, policy, agw) {
		for _, gateway := range lookupGatewaysForPolicyTarget(ctx, policy.Namespace, target, agw, nil) {
			gateways[gateway] = struct{}{}
		}
	}
	if len(gateways) == 0 {
		return nil
	}

	source := utils.TypedNamespacedName{
		NamespacedName: types.NamespacedName{
			Namespace: policy.Namespace,
			Name:      policy.Name,
		},
		Kind: wellknown.AgentgatewayPolicyGVK.Kind,
	}

	orderedGateways := make([]types.NamespacedName, 0, len(gateways))
	for gateway := range gateways {
		orderedGateways = append(orderedGateways, gateway)
	}
	sort.SliceStable(orderedGateways, func(i, j int) bool {
		if orderedGateways[i].Namespace != orderedGateways[j].Namespace {
			return orderedGateways[i].Namespace < orderedGateways[j].Namespace
		}
		return orderedGateways[i].Name < orderedGateways[j].Name
	})

	ancestors := make([]*utils.AncestorBackend, 0, len(orderedGateways))
	for _, gateway := range orderedGateways {
		ancestors = append(ancestors, &utils.AncestorBackend{
			Gateway: gateway,
			Backend: *backendRef,
			Source:  source,
		})
	}
	return ancestors
}

func policyBackendTargetRef(namespace string, backendRef gwv1.BackendObjectReference) *utils.TypedNamespacedName {
	refNS := namespace
	if backendRef.Namespace != nil {
		refNS = string(*backendRef.Namespace)
	}

	refKind := wellknown.ServiceKind
	if backendRef.Kind != nil {
		refKind = string(*backendRef.Kind)
	}

	switch refKind {
	case wellknown.ServiceKind, wellknown.AgentgatewayBackendGVK.Kind:
		return &utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{
				Namespace: refNS,
				Name:      string(backendRef.Name),
			},
			Kind: refKind,
		}
	default:
		return nil
	}
}
