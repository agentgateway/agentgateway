package plugins

import (
	"cmp"
	"fmt"
	"strconv"
	"strings"

	"istio.io/istio/pkg/config/schema/gvk"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/translator/sslutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

// NewBackendTLSPlugin creates a new BackendTLSPolicy plugin
func NewBackendTLSPlugin(agw *AgwCollections) AgwPlugin {
	backendTLSTargetIndex := krt.NewIndex(agw.BackendTLSPolicies, "ancestors", func(o *gwv1.BackendTLSPolicy) []utils.TypedNamespacedName {
		return slices.Map(o.Spec.TargetRefs, func(e gwv1.LocalPolicyTargetReferenceWithSectionName) utils.TypedNamespacedName {
			return utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{
					Name:      string(e.Name),
					Namespace: o.Namespace,
				},
				Kind: string(e.Kind),
			}
		})
	})
	backendTLSTarget := backendTLSTargetIndex.AsCollection(append(agw.KrtOpts.ToOptions("agentgateway/BackendTLSPolicyTargets"), utils.TypedNamespacedNameIndexCollectionFunc)...)
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.BackendTLSPolicyGVK.GroupKind(): {
				Build: func(input PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy]) {
					st, o := krt.NewStatusManyCollection(agw.BackendTLSPolicies, func(krtctx krt.HandlerContext, btls *gwv1.BackendTLSPolicy) (*gwv1.PolicyStatus, []AgwPolicy) {
						return translatePoliciesForBackendTLS(krtctx, agw.ControllerName, input.Ancestors, agw.ConfigMaps, agw.Services, backendTLSTarget, btls)
					}, agw.KrtOpts.ToOptions("agentgateway/BackendTLSPolicy")...)
					return convertStatusCollection(st), o
				},
			},
		},
	}
}

// translatePoliciesForService generates backend TLS policies
func translatePoliciesForBackendTLS(
	krtctx krt.HandlerContext,
	controllerName string,
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend],
	cfgmaps krt.Collection[*corev1.ConfigMap],
	svcs krt.Collection[*corev1.Service],
	targetIndex krt.IndexCollection[utils.TypedNamespacedName, *gwv1.BackendTLSPolicy],
	btls *gwv1.BackendTLSPolicy,
) (*gwv1.PolicyStatus, []AgwPolicy) {
	logger := logger.With("plugin_kind", "backendtls")
	var policies []AgwPolicy
	status := btls.Status.DeepCopy()

	// Condition reporting for BackendTLSPolicy is tricky. The references are to Service (or other backends), but we report
	// per-gateway.
	// This means most of the results are aggregated.
	conds := map[string]*condition{
		string(gwv1.PolicyConditionAccepted): {
			reason:  string(gwv1.PolicyReasonAccepted),
			message: "Configuration is valid",
		},
		string(gwv1.BackendTLSPolicyConditionResolvedRefs): {
			reason:  string(gwv1.BackendTLSPolicyReasonResolvedRefs),
			message: "Configuration is valid",
		},
	}

	caCert, err := getBackendTLSCACert(krtctx, cfgmaps, btls, conds)
	if err != nil {
		conds[string(gwv1.PolicyConditionAccepted)].error = &ConfigError{
			Reason:  string(gwv1.BackendTLSPolicyReasonNoValidCACertificate),
			Message: err.Error(),
		}
		caCert = dummyCaCert
	}
	sans := slices.MapFilter(btls.Spec.Validation.SubjectAltNames, func(e gwv1.SubjectAltName) *string {
		switch e.Type {
		case gwv1.HostnameSubjectAltNameType:
			return ptr.Of(string(e.Hostname))
		case gwv1.URISubjectAltNameType:
			return ptr.Of(string(e.URI))
		}
		return nil
	})

	// Ideally we would report status for an unknown reference. However, Gateway API has decided we should report 1 status
	// per Gateway, instead of per-Backend. This is questionable for users, but also means we don't have to worry about
	// telling users if a reference is invalid and should just silently fail...
	uniqueGateways := sets.New[types.NamespacedName]()
	for _, target := range btls.Spec.TargetRefs {
		var policyTarget *api.PolicyTarget

		tgtRef := utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{
				Name:      string(target.Name),
				Namespace: btls.Namespace,
			},
			Kind: string(target.Kind),
		}

		ancestorBackends := krt.Fetch(krtctx, ancestors, krt.FilterKey(tgtRef.String()))
		for _, gwl := range ancestorBackends {
			for _, i := range gwl.Objects {
				uniqueGateways.Insert(i.Gateway)
			}
		}

		backendTLSPoliciesForThisTarget := krt.FetchOne(krtctx, targetIndex, krt.FilterKey(tgtRef.String()))
		if backendTLSPoliciesForThisTarget != nil {
			if err := checkConflicted(btls, target, backendTLSPoliciesForThisTarget); err != nil {
				conds[string(gwv1.PolicyConditionAccepted)].error = &ConfigError{
					Reason:  string(gwv1.PolicyReasonConflicted),
					Message: err.Error(),
				}
				// We cannot send this policy to agentgateway, as it would not know the priority logic.
				continue
			}
		}

		switch string(target.Kind) {
		case wellknown.AgentgatewayBackendGVK.Kind:
			policyTarget = &api.PolicyTarget{
				Kind: utils.BackendTarget(btls.Namespace, string(target.Name), target.SectionName),
			}
		case wellknown.ServiceKind:
			// BackendTLSPolicy supports named port sectionName (unfortunately)
			policyTarget = &api.PolicyTarget{
				Kind: utils.ServiceTarget(btls.Namespace, string(target.Name), (*string)(target.SectionName)),
			}
			// It is a named port, attempt to lookup
			if sn := target.SectionName; sn != nil {
				_, convErr := strconv.Atoi(string(*sn))
				if convErr != nil {
					svc := ptr.Flatten(krt.FetchOne(krtctx, svcs, krt.FilterObjectName(tgtRef.NamespacedName)))
					if svc != nil {
						for _, p := range svc.Spec.Ports {
							if p.Name == string(*sn) {
								policyTarget = &api.PolicyTarget{
									Kind: utils.ServicePortTarget(btls.Namespace, string(target.Name), uint32(p.Port)), // nolint:gosec // G115: kubebuilder validation ensures safe for uint32
								}
								break
							}
						}
					}
				}
			}
		case wellknown.InferencePoolKind:
			policyTarget = &api.PolicyTarget{
				Kind: utils.InferencePoolTarget(btls.Namespace, string(target.Name), (*string)(target.SectionName)),
			}
		default:
			logger.Warn("unsupported target kind", "kind", target.Kind, "policy", btls.Name)
			continue
		}

		policy := &api.Policy{
			Key:    btls.Namespace + "/" + btls.Name + backendTlsPolicySuffix + attachmentName(policyTarget),
			Name:   TypedResourceName(wellknown.BackendTLSPolicyKind, btls),
			Target: policyTarget,
			Kind: &api.Policy_Backend{
				Backend: &api.BackendPolicySpec{
					Kind: &api.BackendPolicySpec_BackendTls{
						BackendTls: &api.BackendPolicySpec_BackendTLS{
							Root: caCert,
							// Used for mTLS, not part of the spec currently
							Cert: nil,
							Key:  nil,
							// Validation.Hostname is a required value and validated with CEL
							Hostname:              ptr.Of(string(btls.Spec.Validation.Hostname)),
							VerifySubjectAltNames: sans,
						},
					},
				},
			},
		}
		policies = append(policies, AgwPolicy{policy})
	}
	ancestorStatus := make([]gwv1.PolicyAncestorStatus, 0, len(btls.Spec.TargetRefs))
	for g := range uniqueGateways {
		pr := gwv1.ParentReference{
			Group: ptr.Of(gwv1.Group(gvk.KubernetesGateway.Group)),
			Kind:  ptr.Of(gwv1.Kind(gvk.KubernetesGateway.Kind)),
			Name:  gwv1.ObjectName(g.Name),
		}
		ancestorStatus = append(ancestorStatus, setAncestorStatus(pr, status, btls.Generation, conds, gwv1.GatewayController(controllerName)))
	}
	status.Ancestors = mergeAncestors(controllerName, status.Ancestors, ancestorStatus)
	return status, policies
}

// checkConflicted verifies if this target for this BackendTLSPolicy is conflicted.
// Conflicted means there is a different BackendTLSPolicy, with the same target, and the other one is a higher priority.
// Note: allMatches doesn't filter by sectionName, so we do that here.
func checkConflicted(
	btls *gwv1.BackendTLSPolicy,
	target gwv1.LocalPolicyTargetReferenceWithSectionName,
	allMatches *krt.IndexObject[utils.TypedNamespacedName, *gwv1.BackendTLSPolicy],
) error {
	for _, m := range allMatches.Objects {
		if m.UID == btls.UID {
			// This is ourself, skip it
			continue
		}
		conflict := slices.FindFunc(m.Spec.TargetRefs, func(name gwv1.LocalPolicyTargetReferenceWithSectionName) bool {
			return targetEqual(target, name)
		})
		if conflict == nil {
			continue
		}
		// If the one we match with is higher priority, we are conflicted
		if comparePolicy(m, btls) {
			return fmt.Errorf("policy %v matches the same target but with higher priority", m.Name)
		}
	}
	return nil
}

// comparePolicy compares two objects, and returns true if the first is a higher priority than the second.
// Priority is determined by creation timestamp and alphabetical order
func comparePolicy(a, b metav1.Object) bool {
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

func targetEqual(a, b gwv1.LocalPolicyTargetReferenceWithSectionName) bool {
	return a.Group == b.Group &&
		a.Kind == b.Kind &&
		a.Name == b.Name &&
		ptr.Equal(a.SectionName, b.SectionName)
}

// a sentinel value to send to agentgateway to signal that it should reject TLS connects due to invalid config
var dummyCaCert = []byte("invalid")

func getBackendTLSCACert(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	btls *gwv1.BackendTLSPolicy,
	conds map[string]*condition,
) ([]byte, error) {
	validation := btls.Spec.Validation
	if wk := validation.WellKnownCACertificates; wk != nil {
		switch kind := *wk; kind {
		case gwv1.WellKnownCACertificatesSystem:
			return nil, nil

		default:
			conds[string(gwv1.PolicyConditionAccepted)].error = &ConfigError{
				Reason:  string(gwv1.PolicyReasonInvalid),
				Message: fmt.Sprintf("Unknown wellKnownCACertificates: %v", *wk),
			}
			return nil, fmt.Errorf("unknown wellKnownCACertificates: %v", *wk)
		}
	}

	// One of WellKnownCACertificates or CACertificateRefs will always be specified (CEL validated)
	if len(validation.CACertificateRefs) == 0 {
		// should never happen as this is CEL validated. Only here to prevent panic in tests
		return nil, fmt.Errorf("no CACertificateRefs specified")
	}

	var sb strings.Builder
	for _, ref := range validation.CACertificateRefs {
		if ref.Group != gwv1.Group(wellknown.ConfigMapGVK.Group) || ref.Kind != gwv1.Kind(wellknown.ConfigMapGVK.Kind) {
			conds[string(gwv1.BackendTLSPolicyReasonResolvedRefs)].error = &ConfigError{
				Reason:  string(gwv1.BackendTLSPolicyReasonInvalidKind),
				Message: "Certificate reference invalid: " + string(ref.Kind),
			}
			return nil, fmt.Errorf("invalid certificate reference: %v", ref)
		}
		nn := types.NamespacedName{
			Name:      string(ref.Name),
			Namespace: btls.Namespace,
		}
		cfgmap := krt.FetchOne(krtctx, cfgmaps, krt.FilterObjectName(nn))
		if cfgmap == nil {
			conds[string(gwv1.BackendTLSPolicyReasonResolvedRefs)].error = &ConfigError{
				Reason:  string(gwv1.BackendTLSPolicyReasonInvalidCACertificateRef),
				Message: "Certificate reference not found",
			}
			return nil, fmt.Errorf("certificate reference not found: %v", ref)
		}
		caCert, err := sslutils.GetCACertFromConfigMap(ptr.Flatten(cfgmap))
		if err != nil {
			conds[string(gwv1.BackendTLSPolicyReasonResolvedRefs)].error = &ConfigError{
				Reason:  string(gwv1.BackendTLSPolicyReasonInvalidCACertificateRef),
				Message: "Certificate invalid: " + err.Error(),
			}
			return nil, fmt.Errorf("certificate invalid: %v", err)
		}
		if sb.Len() > 0 {
			sb.WriteString("\n")
		}
		sb.WriteString(caCert)
	}
	return []byte(sb.String()), nil
}
