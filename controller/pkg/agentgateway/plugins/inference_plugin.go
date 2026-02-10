package plugins

import (
	"fmt"
	"strconv"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	meta "k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
)

const (
	defaultInferencePoolStatusKind = "Status"
	defaultInferencePoolStatusName = "default"
)

// NewInferencePlugin creates a new InferencePool policy plugin
func NewInferencePlugin(agw *AgwCollections) AgwPlugin {
	status, policyCol := krt.NewStatusManyCollection(agw.InferencePools, func(krtctx krt.HandlerContext, infPool *inf.InferencePool) (*inf.InferencePoolStatus, []AgwPolicy) {
		return translatePoliciesForInferencePool(infPool, agw.ControllerName)
	}, agw.KrtOpts.ToOptions("agentgateway/InferencePoolsPolicy")...)
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.InferencePoolGVK.GroupKind(): {
				Build: func(input PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy]) {
					return convertStatusCollection(status), policyCol
				},
			},
		},
	}
}

// translatePoliciesForInferencePool generates policies for a single inference pool
func translatePoliciesForInferencePool(pool *inf.InferencePool, controllerName string) (*inf.InferencePoolStatus, []AgwPolicy) {
	var infPolicies []AgwPolicy
	status := pool.Status.DeepCopy()
	if status == nil {
		status = &inf.InferencePoolStatus{}
	}
	if len(status.Parents) == 0 {
		status.Parents = []inf.ParentStatus{{
			ParentRef: inf.ParentReference{
				Kind: inf.Kind(defaultInferencePoolStatusKind),
				Name: inf.ObjectName(defaultInferencePoolStatusName),
			},
		}}
	}

	// 'service/{namespace}/{hostname}:{port}'
	hostname := kubeutils.GetInferenceServiceHostname(pool.Name, pool.Namespace)

	epr := pool.Spec.EndpointPickerRef
	validationErr := validateInferencePoolEndpointPickerRef(epr)
	for i := range status.Parents {
		if controllerName != "" {
			status.Parents[i].ControllerName = inf.ControllerName(controllerName)
		}
		meta.SetStatusCondition(&status.Parents[i].Conditions, buildInferencePoolAcceptedCondition(pool.Generation, controllerName))
		meta.SetStatusCondition(&status.Parents[i].Conditions, buildInferencePoolResolvedRefsCondition(pool.Generation, validationErr))
	}
	if validationErr != nil {
		logger.Warn("inference pool endpoint picker ref invalid, skipping", "pool", pool.Name, "error", validationErr)
		return status, nil
	}

	eppPort := epr.Port.Number

	eppSvc := kubeutils.GetServiceHostname(string(epr.Name), pool.Namespace)

	failureMode := api.BackendPolicySpec_InferenceRouting_FAIL_CLOSED
	if epr.FailureMode == inf.EndpointPickerFailOpen {
		failureMode = api.BackendPolicySpec_InferenceRouting_FAIL_OPEN
	}

	// Create the inference routing policy
	inferencePolicy := &api.Policy{
		Key:    pool.Namespace + "/" + pool.Name + ":inference",
		Name:   TypedResourceName(wellknown.InferencePoolGVK.Kind, pool),
		Target: &api.PolicyTarget{Kind: utils.ServiceTargetWithHostname(pool.Namespace, hostname, nil)},
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_InferenceRouting_{
					InferenceRouting: &api.BackendPolicySpec_InferenceRouting{
						EndpointPicker: &api.BackendReference{
							Kind: &api.BackendReference_Service_{
								Service: &api.BackendReference_Service{
									Hostname:  eppSvc,
									Namespace: pool.Namespace,
								},
							},
							Port: uint32(eppPort), //nolint:gosec // G115: eppPort is derived from validated port numbers
						},
						FailureMode: failureMode,
					},
				},
			},
		},
	}
	infPolicies = append(infPolicies, AgwPolicy{Policy: inferencePolicy})

	// Create the TLS policy for the endpoint picker
	// TODO: we would want some way if they explicitly set a BackendTLSPolicy for the EPP to respect that
	inferencePolicyTLS := &api.Policy{
		Key:    pool.Namespace + "/" + pool.Name + ":inferencetls",
		Name:   TypedResourceName(wellknown.InferencePoolGVK.Kind, pool),
		Target: &api.PolicyTarget{Kind: utils.ServiceTargetWithHostname(pool.Namespace, eppSvc, ptr.Of(strconv.Itoa(int(eppPort))))},
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_BackendTls{
					BackendTls: &api.BackendPolicySpec_BackendTLS{
						// The spec mandates this :vomit:
						Verification: api.BackendPolicySpec_BackendTLS_INSECURE_ALL,
					},
				},
			},
		},
	}
	infPolicies = append(infPolicies, AgwPolicy{Policy: inferencePolicyTLS})

	logger.Debug("generated inference pool policies",
		"pool", pool.Name,
		"namespace", pool.Namespace,
		"inference_policy", inferencePolicy.Name,
		"tls_policy", inferencePolicyTLS.Name)

	return status, infPolicies
}

func validateInferencePoolEndpointPickerRef(epr inf.EndpointPickerRef) error {
	if epr.Group != nil && *epr.Group != "" {
		return fmt.Errorf("endpointPickerRef.group must be empty, got %q", *epr.Group)
	}

	kind := epr.Kind
	if kind == "" {
		// InferencePool defaults this field to Service.
		kind = inf.Kind(wellknown.ServiceKind)
	}
	if kind != inf.Kind(wellknown.ServiceKind) {
		return fmt.Errorf("endpointPickerRef.kind must be %q, got %q", wellknown.ServiceKind, kind)
	}

	if epr.Port == nil {
		return fmt.Errorf("endpointPickerRef.port must be specified")
	}
	return nil
}

func buildInferencePoolAcceptedCondition(gen int64, controllerName string) metav1.Condition {
	msg := "InferencePool has been accepted"
	if controllerName != "" {
		msg = fmt.Sprintf("InferencePool has been accepted by controller %s", controllerName)
	}
	return metav1.Condition{
		Type:               string(inf.InferencePoolConditionAccepted),
		Status:             metav1.ConditionTrue,
		Reason:             string(inf.InferencePoolReasonAccepted),
		Message:            msg,
		ObservedGeneration: gen,
		LastTransitionTime: metav1.Now(),
	}
}

func buildInferencePoolResolvedRefsCondition(gen int64, validationErr error) metav1.Condition {
	cond := metav1.Condition{
		Type:               string(inf.InferencePoolConditionResolvedRefs),
		ObservedGeneration: gen,
		LastTransitionTime: metav1.Now(),
	}
	if validationErr == nil {
		cond.Status = metav1.ConditionTrue
		cond.Reason = string(inf.InferencePoolReasonResolvedRefs)
		cond.Message = "All InferencePool references have been resolved"
		return cond
	}

	cond.Status = metav1.ConditionFalse
	cond.Reason = string(inf.InferencePoolReasonInvalidExtensionRef)
	cond.Message = "error: " + validationErr.Error()
	return cond
}
