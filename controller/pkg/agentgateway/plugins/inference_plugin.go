package plugins

import (
	"fmt"
	"sort"
	"strconv"
	"strings"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

const (
	defaultInferencePoolStatusKind = "Status"
	defaultInferencePoolStatusName = "default"
)

// NewInferencePlugin creates a new InferencePool policy plugin
func NewInferencePlugin(agw *AgwCollections) AgwPlugin {
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.InferencePoolGVK.GroupKind(): {
				Build: func(input PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy]) {
					status, policyCol := krt.NewStatusManyCollection(agw.InferencePools, func(krtctx krt.HandlerContext, infPool *inf.InferencePool) (*inf.InferencePoolStatus, []AgwPolicy) {
						return translatePoliciesForInferencePool(krtctx, agw.ControllerName, input.Ancestors, agw.Services, infPool)
					}, agw.KrtOpts.ToOptions("agentgateway/InferencePools")...)
					return convertStatusCollection(status), policyCol
				},
			},
		},
	}
}

// translatePoliciesForInferencePool generates policies for a single inference pool.
func translatePoliciesForInferencePool(
	krtctx krt.HandlerContext,
	controllerName string,
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend],
	services krt.Collection[*corev1.Service],
	pool *inf.InferencePool,
) (*inf.InferencePoolStatus, []AgwPolicy) {
	var infPolicies []AgwPolicy

	epr := pool.Spec.EndpointPickerRef
	validationErr := validateInferencePoolEndpointPickerRef(krtctx, pool, services)
	attachedGateways := inferencePoolAttachedGateways(krtctx, ancestors, pool)
	status := buildInferencePoolStatus(pool, controllerName, attachedGateways, validationErr)

	// 'service/{namespace}/{hostname}:{port}'
	hostname := kubeutils.GetInferenceServiceHostname(pool.Name, pool.Namespace)
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

func validateInferencePoolEndpointPickerRef(krtctx krt.HandlerContext, pool *inf.InferencePool, services krt.Collection[*corev1.Service]) error {
	epr := pool.Spec.EndpointPickerRef
	var errs []string

	if epr.Group != nil && *epr.Group != "" {
		errs = append(errs, fmt.Sprintf("endpointPickerRef.group must be empty, got %q", *epr.Group))
	}

	kind := epr.Kind
	if kind == "" {
		// InferencePool defaults this field to Service.
		kind = inf.Kind(wellknown.ServiceKind)
	}
	if kind != inf.Kind(wellknown.ServiceKind) {
		errs = append(errs, fmt.Sprintf("endpointPickerRef.kind must be %q, got %q", wellknown.ServiceKind, kind))
	}

	// InferencePool v1 only supports a single target port.
	if len(pool.Spec.TargetPorts) != 1 {
		errs = append(errs, "inferencePool.targetPorts must contain exactly one entry")
	}

	if epr.Port == nil {
		errs = append(errs, "endpointPickerRef.port must be specified")
		return inferencePoolValidationError(errs)
	}

	svc := ptr.Flatten(krt.FetchOne(krtctx, services, krt.FilterKey(types.NamespacedName{Namespace: pool.Namespace, Name: string(epr.Name)}.String())))
	if svc == nil {
		errs = append(errs, fmt.Sprintf("endpointPickerRef Service %s/%s not found", pool.Namespace, epr.Name))
		return inferencePoolValidationError(errs)
	}

	if svc.Spec.Type == corev1.ServiceTypeExternalName {
		errs = append(errs, "endpointPickerRef Service must not be ExternalName")
	}

	// Service must expose the requested TCP port.
	foundTCPPort := false
	eppPort := int32(epr.Port.Number)
	for _, sp := range svc.Spec.Ports {
		proto := sp.Protocol
		if proto == "" {
			proto = corev1.ProtocolTCP
		}
		if sp.Port == eppPort && proto == corev1.ProtocolTCP {
			foundTCPPort = true
			break
		}
	}
	if !foundTCPPort {
		errs = append(errs, fmt.Sprintf("endpointPickerRef.port %d must reference a TCP Service port on %s/%s", eppPort, pool.Namespace, epr.Name))
	}

	return inferencePoolValidationError(errs)
}

func inferencePoolValidationError(errs []string) error {
	if len(errs) == 0 {
		return nil
	}
	return fmt.Errorf("%s", strings.Join(errs, "; "))
}

func inferencePoolAttachedGateways(
	krtctx krt.HandlerContext,
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend],
	pool *inf.InferencePool,
) map[types.NamespacedName]struct{} {
	gateways := make(map[types.NamespacedName]struct{})
	if ancestors == nil {
		return gateways
	}

	targetRef := utils.TypedNamespacedName{
		NamespacedName: types.NamespacedName{
			Name:      pool.Name,
			Namespace: pool.Namespace,
		},
		Kind: wellknown.InferencePoolKind,
	}

	ancestorBackends := krt.Fetch(krtctx, ancestors, krt.FilterKey(targetRef.String()))
	for _, gwl := range ancestorBackends {
		for _, i := range gwl.Objects {
			gateways[i.Gateway] = struct{}{}
		}
	}
	return gateways
}

func buildInferencePoolStatus(
	pool *inf.InferencePool,
	controllerName string,
	attachedGateways map[types.NamespacedName]struct{},
	validationErr error,
) *inf.InferencePoolStatus {
	status := pool.Status.DeepCopy()
	if status == nil {
		status = &inf.InferencePoolStatus{}
	}

	existingOurs := make(map[string]inf.ParentStatus)
	mergedParents := make([]inf.ParentStatus, 0, len(status.Parents)+len(attachedGateways)+1)
	for _, p := range status.Parents {
		if string(p.ControllerName) != controllerName {
			mergedParents = append(mergedParents, p)
			continue
		}
		existingOurs[inferencePoolParentMergeKey(p.ParentRef)] = p
	}

	conditions := inferencePoolConditionMap(controllerName, validationErr)
	for _, ref := range desiredInferencePoolParentRefs(attachedGateways, validationErr) {
		existingConds := []metav1.Condition(nil)
		if existing, found := existingOurs[inferencePoolParentMergeKey(ref)]; found {
			existingConds = existing.Conditions
		}
		mergedParents = append(mergedParents, inf.ParentStatus{
			ParentRef:      ref,
			ControllerName: inf.ControllerName(controllerName),
			Conditions:     setConditions(pool.Generation, existingConds, conditions),
		})
	}

	status.Parents = mergedParents
	return status
}

func desiredInferencePoolParentRefs(attachedGateways map[types.NamespacedName]struct{}, err error) []inf.ParentReference {
	if len(attachedGateways) == 0 {
		if err == nil {
			return []inf.ParentReference{}
		}
		return []inf.ParentReference{{
			Kind: inf.Kind(defaultInferencePoolStatusKind),
			Name: inf.ObjectName(defaultInferencePoolStatusName),
		}}
	}

	gateways := make([]types.NamespacedName, 0, len(attachedGateways))
	for g := range attachedGateways {
		gateways = append(gateways, g)
	}
	sort.SliceStable(gateways, func(i, j int) bool {
		if gateways[i].Namespace == gateways[j].Namespace {
			return gateways[i].Name < gateways[j].Name
		}
		return gateways[i].Namespace < gateways[j].Namespace
	})

	refs := make([]inf.ParentReference, 0, len(gateways))
	for _, g := range gateways {
		refs = append(refs, inf.ParentReference{
			Group:     ptr.Of(inf.Group(wellknown.GatewayGroup)),
			Kind:      inf.Kind(wellknown.GatewayKind),
			Namespace: inf.Namespace(g.Namespace),
			Name:      inf.ObjectName(g.Name),
		})
	}
	return refs
}

func inferencePoolConditionMap(controllerName string, validationErr error) map[string]*condition {
	msg := "InferencePool has been accepted"
	if controllerName != "" {
		msg = fmt.Sprintf("InferencePool has been accepted by controller %s", controllerName)
	}

	conds := map[string]*condition{
		string(inf.InferencePoolConditionAccepted): {
			reason:  string(inf.InferencePoolReasonAccepted),
			message: msg,
		},
		string(inf.InferencePoolConditionResolvedRefs): {
			reason:  string(inf.InferencePoolReasonResolvedRefs),
			message: "All InferencePool references have been resolved",
		},
	}
	if validationErr != nil {
		conds[string(inf.InferencePoolConditionResolvedRefs)].error = &ConfigError{
			Reason:  string(inf.InferencePoolReasonInvalidExtensionRef),
			Message: "error: " + validationErr.Error(),
		}
	}
	return conds
}

func inferencePoolParentMergeKey(ref inf.ParentReference) string {
	kind := string(ref.Kind)
	if kind == "" {
		kind = wellknown.GatewayKind
	}

	group := ""
	if ref.Group != nil && *ref.Group != "" {
		group = string(*ref.Group)
	} else if kind == wellknown.GatewayKind {
		// For Gateway parent refs, API defaulting implies gateway.networking.k8s.io.
		group = wellknown.GatewayGroup
	}
	return fmt.Sprintf("%s/%s/%s/%s", group, kind, ref.Namespace, ref.Name)
}
