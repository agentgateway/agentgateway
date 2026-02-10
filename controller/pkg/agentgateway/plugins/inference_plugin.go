package plugins

import (
	"strconv"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/runtime/schema"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/utils"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

// NewInferencePlugin creates a new InferencePool policy plugin
func NewInferencePlugin(agw *AgwCollections) AgwPlugin {
	policyCol := krt.NewManyCollection(agw.InferencePools, func(krtctx krt.HandlerContext, infPool *inf.InferencePool) []AgwPolicy {
		return translatePoliciesForInferencePool(infPool)
	})
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.InferencePoolGVK.GroupKind(): {
				Policies: policyCol,
			},
		},
		ExtraHasSynced: func() bool {
			return policyCol.HasSynced()
		},
	}
}

// translatePoliciesForInferencePool generates policies for a single inference pool
func translatePoliciesForInferencePool(pool *inf.InferencePool) []AgwPolicy {
	var infPolicies []AgwPolicy

	// 'service/{namespace}/{hostname}:{port}'
	hostname := kubeutils.GetInferenceServiceHostname(pool.Name, pool.Namespace)

	epr := pool.Spec.EndpointPickerRef
	if epr.Group != nil && *epr.Group != "" {
		logger.Warn("inference pool endpoint picker ref has non-empty group, skipping", "pool", pool.Name, "group", *epr.Group)
		return nil
	}

	if epr.Kind != wellknown.ServiceKind {
		logger.Warn("inference pool extension ref is not a Service, skipping", "pool", pool.Name, "kind", epr.Kind)
		return nil
	}

	if epr.Port == nil {
		logger.Warn("inference pool extension ref port must be specified, skipping", "pool", pool.Name, "kind", epr.Kind)
		return nil
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

	return infPolicies
}
