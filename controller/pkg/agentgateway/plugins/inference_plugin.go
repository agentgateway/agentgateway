package plugins

import (
	"encoding/json"
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
	bbr "sigs.k8s.io/gateway-api-inference-extension/pkg/bbr/plugins/basemodelextractor"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const (
	defaultInferencePoolStatusKind = "Status"
	defaultInferencePoolStatusName = "default"

	virtualModelsPath = "/v1/models"
)

// NewInferencePlugin creates a new InferencePool policy plugin
func NewInferencePlugin(agw *AgwCollections) AgwPlugin {
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.InferencePoolGVK.GroupKind(): {
				Build: func(input PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy]) {
					status, policyCol := krt.NewStatusManyCollection(agw.InferencePools, func(krtctx krt.HandlerContext, infPool *inf.InferencePool) (*inf.InferencePoolStatus, []AgwPolicy) {
						return translatePoliciesForInferencePool(krtctx, agw.ControllerName, input.References, agw.Services, infPool)
					}, agw.KrtOpts.ToOptions("agentgateway/InferencePools")...)
					return ConvertStatusCollection(status), policyCol
				},
			},
		},
		AddResourceExtension: &AddResourcesPlugin{
			Routes: krt.NewManyCollection(agw.Gateways, func(krtctx krt.HandlerContext, gw *gwv1.Gateway) []ir.AgwResource {
				gwNN := types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name}
				models := collectInferenceModelsForGateway(krtctx, agw, gwNN)
				if len(models) == 0 {
					return nil
				}
				return buildModelsRoutes(gw, gwNN, models)
			}, agw.KrtOpts.ToOptions("agentgateway/VirtualModels")...),
		},
	}
}

// translatePoliciesForInferencePool generates policies for a single inference pool.
func translatePoliciesForInferencePool(
	krtctx krt.HandlerContext,
	controllerName string,
	references ReferenceIndex,
	services krt.Collection[*corev1.Service],
	pool *inf.InferencePool,
) (*inf.InferencePoolStatus, []AgwPolicy) {
	var infPolicies []AgwPolicy

	epr := pool.Spec.EndpointPickerRef
	validationErr := validateInferencePoolEndpointPickerRef(krtctx, pool, services)
	attachedGateways := inferencePoolAttachedGateways(krtctx, references, pool)
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
	gatewayTargets := make([]types.NamespacedName, 0, len(attachedGateways))
	for gatewayTarget := range attachedGateways {
		gatewayTargets = append(gatewayTargets, gatewayTarget)
	}
	infPolicies = appendPolicyForGateways(infPolicies, gatewayTargets, inferencePolicy)

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
	infPolicies = appendPolicyForGateways(infPolicies, gatewayTargets, inferencePolicyTLS)

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
		kind = wellknown.ServiceKind
	}
	if kind != wellknown.ServiceKind {
		errs = append(errs, fmt.Sprintf("endpointPickerRef.kind must be %q, got %q", wellknown.ServiceKind, kind))
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
	references ReferenceIndex,
	pool *inf.InferencePool,
) map[types.NamespacedName]struct{} {
	gateways := make(map[types.NamespacedName]struct{})

	targetRef := utils.TypedNamespacedName{
		NamespacedName: types.NamespacedName{
			Name:      pool.Name,
			Namespace: pool.Namespace,
		},
		Kind: wellknown.InferencePoolKind,
	}

	for gateway := range references.LookupGatewaysForBackend(krtctx, targetRef) {
		gateways[gateway] = struct{}{}
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
			Kind: defaultInferencePoolStatusKind,
			Name: defaultInferencePoolStatusName,
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
			Kind:      wellknown.GatewayKind,
			Namespace: inf.Namespace(g.Namespace),
			Name:      inf.ObjectName(g.Name),
		})
	}
	return refs
}

func inferencePoolConditionMap(controllerName string, validationErr error) map[string]*Condition {
	msg := "InferencePool has been accepted"
	if controllerName != "" {
		msg = fmt.Sprintf("InferencePool has been accepted by controller %s", controllerName)
	}

	conds := map[string]*Condition{
		string(inf.InferencePoolConditionAccepted): {
			Reason:  string(inf.InferencePoolReasonAccepted),
			Message: msg,
		},
		string(inf.InferencePoolConditionResolvedRefs): {
			Reason:  string(inf.InferencePoolReasonResolvedRefs),
			Message: "All InferencePool references have been resolved",
		},
	}
	if validationErr != nil {
		conds[string(inf.InferencePoolConditionResolvedRefs)].Error = &ConfigError{
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

// modelEntry is the per-model entry in the OpenAI /v1/models response.
type modelEntry struct {
	ID     string `json:"id"`
	Object string `json:"object"`
}

// collectInferenceModelsForGateway scans all HTTPRoutes looking for rules that
// (a) reference at least one InferencePool as a backend, and
// (b) carry an exact-match header condition on X-Gateway-Base-Model-Name.
//
// It returns a deduplicated, sorted slice of model names found for the given gateway.
func collectInferenceModelsForGateway(krtctx krt.HandlerContext, agw *AgwCollections, gwNN types.NamespacedName) []string {
	allRoutes := krt.Fetch(krtctx, agw.HTTPRoutes)
	seen := make(map[string]struct{})
	for _, route := range allRoutes {
		if !routeAttachedToGateway(route, gwNN) {
			continue
		}
		for _, rule := range route.Spec.Rules {
			if !ruleHasInferencePoolBackend(rule) {
				continue
			}
			for _, match := range rule.Matches {
				for _, h := range match.Headers {
					if string(h.Name) == bbr.BaseModelHeader &&
						(h.Type == nil || *h.Type == gwv1.HeaderMatchExact) {
						seen[h.Value] = struct{}{}
					}
				}
			}
		}
	}
	if len(seen) == 0 {
		return nil
	}
	models := make([]string, 0, len(seen))
	for m := range seen {
		models = append(models, m)
	}
	sort.Strings(models)
	return models
}

// routeAttachedToGateway reports whether the HTTPRoute has a parentRef pointing
// at the given Gateway.
func routeAttachedToGateway(route *gwv1.HTTPRoute, gwNN types.NamespacedName) bool {
	for _, ref := range route.Spec.ParentRefs {
		if ref.Kind != nil && string(*ref.Kind) != wellknown.GatewayKind {
			continue
		}
		if ref.Group != nil && string(*ref.Group) != wellknown.GatewayGroup {
			continue
		}
		ns := route.Namespace
		if ref.Namespace != nil {
			ns = string(*ref.Namespace)
		}
		if ns == gwNN.Namespace && string(ref.Name) == gwNN.Name {
			return true
		}
	}
	return false
}

// ruleHasInferencePoolBackend returns true if any backendRef in the rule
// references an InferencePool.
func ruleHasInferencePoolBackend(rule gwv1.HTTPRouteRule) bool {
	for _, backend := range rule.BackendRefs {
		gk := schema.GroupKind{
			Group: string(ptr.OrDefault((*gwv1.Group)(backend.Group), gwv1.Group(""))),
			Kind:  string(ptr.OrDefault((*gwv1.Kind)(backend.Kind), gwv1.Kind("Service"))),
		}
		if gk == wellknown.InferencePoolGVK.GroupKind() {
			return true
		}
	}
	return false
}

// buildModelsRoutes creates one ir.AgwResource per Gateway listener.
// Each resource is an api.Route for GET /v1/models that returns the aggregated
// model list via an inline directResponse TrafficPolicy.
func buildModelsRoutes(gw *gwv1.Gateway, gwNN types.NamespacedName, models []string) []ir.AgwResource {
	body := buildModelsJSON(models)

	var resources []ir.AgwResource
	for _, listener := range gw.Spec.Listeners {
		listenerKey := utils.InternalGatewayName(gwNN.Namespace, gwNN.Name, string(listener.Name))
		routeKey := gwNN.Namespace + "/" + gwNN.Name + "." + string(listener.Name) + ":virtual-models"

		route := &api.Route{
			Key:         routeKey,
			ListenerKey: listenerKey,
			Name: &api.RouteName{
				Kind:      wellknown.InferencePoolGVK.Kind,
				Namespace: gwNN.Namespace,
				Name:      gwNN.Name,
			},
			Matches: []*api.RouteMatch{
				{
					Path:   &api.PathMatch{Kind: &api.PathMatch_Exact{Exact: virtualModelsPath}},
					Method: &api.MethodMatch{Exact: "GET"},
				},
			},
			TrafficPolicies: []*api.TrafficPolicySpec{
				{
					Kind: &api.TrafficPolicySpec_DirectResponse{
						DirectResponse: &api.DirectResponse{
							Status: 200,
							Body:   body,
						},
					},
				},
				{
					Kind: &api.TrafficPolicySpec_Transformation{
						Transformation: &api.TrafficPolicySpec_TransformationPolicy{
							Response: &api.TrafficPolicySpec_TransformationPolicy_Transform{
								Set: []*api.TrafficPolicySpec_HeaderTransformation{
									{
										Name:       "Content-Type",
										Expression: "'application/json'",
									},
								},
							},
						},
					},
				},
			},
		}

		resources = append(resources, ir.AgwResource{
			Resource: &api.Resource{Kind: &api.Resource_Route{Route: route}},
			Gateway:  gwNN,
		})
	}
	return resources
}

// buildModelsJSON serialises the model list to an OpenAI-compatible /v1/models
// response body.  Returns the raw JSON bytes; falls back to a minimal valid
// payload if marshalling unexpectedly fails.
func buildModelsJSON(models []string) []byte {
	entries := make([]modelEntry, 0, len(models))
	for _, m := range models {
		entries = append(entries, modelEntry{
			ID:     m,
			Object: "model",
		})
	}
	payload := struct {
		Object string       `json:"object"`
		Data   []modelEntry `json:"data"`
	}{
		Object: "list",
		Data:   entries,
	}
	b, err := json.Marshal(payload)
	if err != nil {
		return []byte(`{"object":"list","data":[]}`)
	}
	return b
}
