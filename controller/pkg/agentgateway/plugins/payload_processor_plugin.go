// POC: PayloadProcessor plugin translates PayloadProcessor CRDs into internal policies.
// This reuses convertTransformSpec() from traffic_plugin.go for CEL validation and conversion.
package plugins

import (
	"errors"
	"fmt"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v0alpha0/ainetworking"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

// POC: GVK for the PayloadProcessor CRD
var PayloadProcessorGVK = schema.GroupVersionKind{
	Group:   ainetworking.GroupName,
	Version: ainetworking.GroupVersion.Version,
	Kind:    "PayloadProcessor",
}

const (
	payloadProcessorPolicySuffix = ":payload-processor" // POC
)

// NewPayloadProcessorPlugin creates a plugin that watches PayloadProcessor CRDs (POC)
func NewPayloadProcessorPlugin(agw *AgwCollections) AgwPlugin {
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			PayloadProcessorGVK.GroupKind(): {
				Build: func(input PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy]) {
					policyCol := krt.NewManyCollection(agw.PayloadProcessors, func(krtctx krt.HandlerContext, pp *ainetworking.PayloadProcessor) []AgwPolicy {
						return translatePayloadProcessor(krtctx, pp, agw, input.References)
					}, agw.KrtOpts.ToOptions("PayloadProcessor/POC")...)
					return nil, policyCol
				},
			},
		},
	}
}

// translatePayloadProcessor converts a PayloadProcessor CRD into AgwPolicy objects (POC)
func translatePayloadProcessor(
	krtctx krt.HandlerContext,
	pp *ainetworking.PayloadProcessor,
	agw *AgwCollections,
	references ReferenceIndex,
) []AgwPolicy {
	ctx := PolicyCtx{
		Krt:        krtctx,
		Collections: agw,
		References: references,
	}
	policies, err := translatePayloadProcessorPolicies(ctx, pp)
	if err != nil {
		logger.Error("error translating PayloadProcessor (POC)", "name", pp.Name, "namespace", pp.Namespace, "error", err)
		return nil
	}

	if len(policies) == 0 {
		return nil
	}

	// POC: For Gateway targets, directly resolve to the gateway's NamespacedName.
	// This is simpler than the full reference resolution used by AgentgatewayPolicy
	// (which goes through PolicyTarget → ReferenceIndex → PolicyAttachments).
	targetRef := pp.Spec.TargetRef
	targetNamespace := pp.Namespace

	var agwPolicies []AgwPolicy

	switch string(targetRef.Kind) {
	case "Gateway":
		// Gateway target: the gateway IS the target
		gatewayNN := types.NamespacedName{
			Namespace: targetNamespace,
			Name:      string(targetRef.Name),
		}
		policyTarget := &api.PolicyTarget{Kind: utils.GatewayTarget(
			targetNamespace,
			string(targetRef.Name),
			targetRef.SectionName,
		)}
		for _, policy := range policies {
			policy.Target = policyTarget
			agwPolicies = append(agwPolicies, AgwPolicy{
				Gateway: &gatewayNN,
				Policy:  policy,
			})
		}
	default:
		// For HTTPRoute/ListenerSet targets, use reference index to find gateways
		targetObject := utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{
				Namespace: targetNamespace,
				Name:      string(targetRef.Name),
			},
			Kind: string(targetRef.Kind),
		}
		policyTarget := buildPolicyTarget(targetRef, targetNamespace)
		gatewayTargets := references.LookupGatewaysForPolicyTarget(krtctx, targetObject, policyTarget).UnsortedList()
		for _, policy := range policies {
			policy.Target = policyTarget
			agwPolicies = appendPolicyForGateways(agwPolicies, gatewayTargets, policy)
		}
	}

	return agwPolicies
}

// translatePayloadProcessorPolicies converts processor entries into api.Policy objects (POC)
func translatePayloadProcessorPolicies(ctx PolicyCtx, pp *ainetworking.PayloadProcessor) ([]*api.Policy, error) {
	var policies []*api.Policy
	var errs []error

	policyName := types.NamespacedName{
		Namespace: pp.Namespace,
		Name:      pp.Name,
	}
	basePolicyName := fmt.Sprintf("%s/%s", pp.Namespace, pp.Name)

	for i, proc := range pp.Spec.Processors {
		if proc.Type == ainetworking.ProcessorTypeExtProc {
			if proc.ExtProc == nil {
				errs = append(errs, fmt.Errorf("processor %q: ExtProc config required for ExtProc type", proc.Name))
				continue
			}

			be, err := buildBackendRef(ctx, proc.ExtProc.BackendRef, pp.Namespace)
			if err != nil {
				errs = append(errs, fmt.Errorf("processor %q: failed to build extProc backendRef: %w", proc.Name, err))
				continue
			}

			failureMode := api.TrafficPolicySpec_ExtProc_FAIL_CLOSED
			if proc.FailureMode == ainetworking.FailureModeOpen {
				failureMode = api.TrafficPolicySpec_ExtProc_FAIL_OPEN
			}

			policyPhase := mapPayloadProcessorPhase(pp.Spec.Phase)

			policy := &api.Policy{
				Key:  fmt.Sprintf("%s:%s[%d]%s", basePolicyName, proc.Name, i, extprocPolicySuffix),
				Name: TypedResourceFromName("PayloadProcessor", policyName),
				Kind: &api.Policy_Traffic{
					Traffic: &api.TrafficPolicySpec{
						Phase: policyPhase,
						Kind: &api.TrafficPolicySpec_ExtProc_{
							ExtProc: &api.TrafficPolicySpec_ExtProc{
								Target:      be,
								FailureMode: failureMode,
							},
						},
					},
				},
			}

			logger.Debug("generated PayloadProcessor ExtProc policy (POC)",
				"processor", proc.Name,
				"phase", pp.Spec.Phase,
				"payloadProcessor", pp.Name)

			policies = append(policies, policy)
			continue
		}

		if proc.Type != ainetworking.ProcessorTypeInProcess || proc.InProcess == nil {
			errs = append(errs, fmt.Errorf("processor %q: InProcess config required for InProcess type", proc.Name))
			continue
		}

		// POC: Convert InProcessTransform to agentgateway Transform for reuse of convertTransformSpec
		transform := inProcessTransformToAgwTransform(&proc.InProcess.Request)
		converted, err := convertTransformSpec(transform)
		if err != nil {
			errs = append(errs, fmt.Errorf("processor %q: %w", proc.Name, err))
			continue
		}

		if converted == nil {
			continue
		}

		// POC: Map phase using the same mapping as AgentgatewayPolicy
		policyPhase := mapPayloadProcessorPhase(pp.Spec.Phase)

		policy := &api.Policy{
			Key:  fmt.Sprintf("%s:%s[%d]%s", basePolicyName, proc.Name, i, payloadProcessorPolicySuffix),
			Name: TypedResourceFromName("PayloadProcessor", policyName),
			Kind: &api.Policy_Traffic{
				Traffic: &api.TrafficPolicySpec{
					Phase: policyPhase,
					Kind: &api.TrafficPolicySpec_Transformation{
						Transformation: &api.TrafficPolicySpec_TransformationPolicy{
							Request: converted,
						},
					},
				},
			},
		}

		logger.Debug("generated PayloadProcessor policy (POC)",
			"processor", proc.Name,
			"phase", pp.Spec.Phase,
			"payloadProcessor", pp.Name)

		policies = append(policies, policy)
	}

	return policies, errors.Join(errs...)
}

// inProcessTransformToAgwTransform converts POC InProcessTransform to agentgateway Transform
// so we can reuse convertTransformSpec() from traffic_plugin.go
func inProcessTransformToAgwTransform(in *ainetworking.InProcessTransform) *agentgateway.Transform {
	if in == nil {
		return nil
	}
	t := &agentgateway.Transform{}

	for _, h := range in.Set {
		t.Set = append(t.Set, agentgateway.HeaderTransformation{
			Name:  agentgateway.HeaderName(h.Name),
			Value: shared.CELExpression(h.Value),
		})
	}
	for _, h := range in.Add {
		t.Add = append(t.Add, agentgateway.HeaderTransformation{
			Name:  agentgateway.HeaderName(h.Name),
			Value: shared.CELExpression(h.Value),
		})
	}
	for _, r := range in.Remove {
		t.Remove = append(t.Remove, agentgateway.HeaderName(r))
	}

	return t
}

// mapPayloadProcessorPhase maps POC phase to internal policy phase (POC)
func mapPayloadProcessorPhase(phase ainetworking.ProcessorPhase) api.TrafficPolicySpec_PolicyPhase {
	switch phase {
	case ainetworking.ProcessorPhasePreRouting:
		return api.TrafficPolicySpec_GATEWAY
	case ainetworking.ProcessorPhasePostRouting:
		return api.TrafficPolicySpec_ROUTE
	default:
		return api.TrafficPolicySpec_ROUTE
	}
}

// buildPolicyTarget creates a policy target from the PayloadProcessor's targetRef (POC)
func buildPolicyTarget(ref shared.LocalPolicyTargetReferenceWithSectionName, namespace string) *api.PolicyTarget {
	switch string(ref.Kind) {
	case "Gateway":
		return &api.PolicyTarget{Kind: utils.GatewayTarget(
			namespace,
			string(ref.Name),
			ref.SectionName,
		)}
	case "HTTPRoute":
		return &api.PolicyTarget{Kind: utils.RouteTarget(
			namespace,
			string(ref.Name),
			"HTTPRoute",
			ref.SectionName,
		)}
	case "ListenerSet":
		return &api.PolicyTarget{Kind: utils.ListenerSetTarget(
			namespace,
			string(ref.Name),
			ref.SectionName,
		)}
	default:
		logger.Warn("unknown targetRef kind for PayloadProcessor (POC)", "kind", ref.Kind)
		return nil
	}
}
