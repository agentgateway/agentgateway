package plugins

import (
	"fmt"
	"log/slog"
	"strconv"
	"strings"

	"istio.io/istio/pkg/config/schema/gvk"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/policyselection"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// BackendTLSTargetBuilder builds an agentgateway policy target for an extension
// kind supported by BackendTLSPolicy.
type BackendTLSTargetBuilder func(namespace string, target gwv1.LocalPolicyTargetReferenceWithSectionName) *api.PolicyTarget

// NewBackendTLSPlugin creates a new BackendTLSPolicy plugin
func NewBackendTLSPlugin(agw *AgwCollections) AgwPlugin {
	return NewBackendTLSPluginWithTargetBuilders(agw, nil)
}

// NewBackendTLSPluginWithTargetBuilders creates a BackendTLSPolicy plugin with
// additional supported target kinds.
func NewBackendTLSPluginWithTargetBuilders(agw *AgwCollections, targetBuilders map[schema.GroupKind]BackendTLSTargetBuilder) AgwPlugin {
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
	backendTLSTarget := backendTLSTargetIndex.AsCollection(append(agw.KrtOpts.ToOptions("policies/BackendTLSPolicyTargets"), utils.TypedNamespacedNameIndexCollectionFunc)...)
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.BackendTLSPolicyGVK.GroupKind(): {
				Build: func(input PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy]) {
					st, o := krt.NewStatusManyCollection(agw.BackendTLSPolicies, func(krtctx krt.HandlerContext, btls *gwv1.BackendTLSPolicy) (*gwv1.PolicyStatus, []AgwPolicy) {
						return translatePoliciesForBackendTLS(krtctx, agw.ControllerName, input.References, agw.ConfigMaps, agw.Secrets, agw.Services, targetBuilders, backendTLSTarget, agw.Gateways, btls)
					}, agw.KrtOpts.ToOptions("policies/BackendTLS")...)
					return ConvertStatusCollection(st, agw.KrtOpts.ToOptions, "policies/BackendTLS"), o
				},
			},
		},
	}
}

// translatePoliciesForService generates backend TLS policies
func translatePoliciesForBackendTLS(
	krtctx krt.HandlerContext,
	controllerName string,
	references ReferenceIndex,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	svcs krt.Collection[*corev1.Service],
	targetBuilders map[schema.GroupKind]BackendTLSTargetBuilder,
	targetIndex krt.IndexCollection[utils.TypedNamespacedName, *gwv1.BackendTLSPolicy],
	gateways krt.Collection[*gwv1.Gateway],
	btls *gwv1.BackendTLSPolicy,
) (*gwv1.PolicyStatus, []AgwPolicy) {
	logger := logger.With("plugin_kind", "backendtls")
	var policies []AgwPolicy
	status := btls.Status.DeepCopy()

	// Condition reporting for BackendTLSPolicy is tricky. The references are to Service (or other backends), but we report
	// per-gateway.
	// This means most of the results are aggregated.
	conds := map[string]*Condition{
		string(gwv1.PolicyConditionAccepted): {
			Reason:  string(gwv1.PolicyReasonAccepted),
			Message: "Configuration is valid",
		},
		string(gwv1.BackendTLSPolicyConditionResolvedRefs): {
			Reason:  string(gwv1.BackendTLSPolicyReasonResolvedRefs),
			Message: "Configuration is valid",
		},
	}

	caCert, err := getBackendTLSCACert(krtctx, cfgmaps, btls, conds)
	if err != nil {
		conds[string(gwv1.PolicyConditionAccepted)].Error = &ConfigError{
			Reason:  string(gwv1.BackendTLSPolicyReasonNoValidCACertificate),
			Message: err.Error(),
		}
		caCert = dummyCaCert
	}
	sans := slices.MapFilter(btls.Spec.Validation.SubjectAltNames, func(e gwv1.SubjectAltName) *string {
		switch e.Type {
		case gwv1.HostnameSubjectAltNameType:
			return new(string(e.Hostname))
		case gwv1.URISubjectAltNameType:
			return new(string(e.URI))
		}
		return nil
	})

	// Ideally we would report status for an unknown reference. However, Gateway API has decided we should report 1 status
	// per Gateway, instead of per-Backend. This is questionable for users, but also means we don't have to worry about
	// telling users if a reference is invalid and should just silently fail...
	uniqueGateways := sets.New[types.NamespacedName]()
	gatewayClientCertErrors := map[types.NamespacedName]*ConfigError{}
	for _, target := range btls.Spec.TargetRefs {
		var policyTarget *api.PolicyTarget

		tgtRef := utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{
				Name:      string(target.Name),
				Namespace: btls.Namespace,
			},
			Kind: string(target.Kind),
		}

		gatewayTargets := references.LookupGatewaysForBackend(krtctx, tgtRef).UnsortedList()
		uniqueGateways = uniqueGateways.InsertAll(gatewayTargets...)

		backendTLSPoliciesForThisTarget := krtutil.FetchIndexObjects(krtctx, targetIndex, tgtRef)
		if err := checkConflicted(btls, target, backendTLSPoliciesForThisTarget); err != nil {
			conds[string(gwv1.PolicyConditionAccepted)].Error = &ConfigError{
				Reason:  string(gwv1.PolicyReasonConflicted),
				Message: err.Error(),
			}
			// We cannot send this policy to agentgateway, as it would not know the priority logic.
			continue
		}

		targetGK := schema.GroupKind{Group: string(target.Group), Kind: string(target.Kind)}
		switch targetGK {
		case wellknown.AgentgatewayBackendGVK.GroupKind():
			policyTarget = &api.PolicyTarget{
				Kind: utils.BackendTarget(btls.Namespace, string(target.Name), target.SectionName),
			}
		case schema.GroupKind{Kind: wellknown.ServiceKind}:
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
		case wellknown.InferencePoolGVK.GroupKind():
			policyTarget = &api.PolicyTarget{
				Kind: utils.InferencePoolTarget(btls.Namespace, string(target.Name), (*string)(target.SectionName)),
			}
		default:
			builder := targetBuilders[targetGK]
			if builder == nil {
				logger.Warn("unsupported target kind", "group", target.Group, "kind", target.Kind, "policy", btls.Name)
				continue
			}
			policyTarget = builder(btls.Namespace, target)
			if policyTarget == nil {
				continue
			}
		}

		baseTLS := &api.BackendPolicySpec_BackendTLS{
			Root: caCert,
			// Validation.Hostname is a required value and validated with CEL
			Hostname:              new(string(btls.Spec.Validation.Hostname)),
			VerifySubjectAltNames: sans,
		}

		for _, gatewayTarget := range gatewayTargets {
			res := &api.BackendPolicySpec_BackendTLS{
				Root:                  baseTLS.Root,
				Hostname:              baseTLS.Hostname,
				VerifySubjectAltNames: baseTLS.VerifySubjectAltNames,
			}
			if err := applyGatewayBackendClientCert(krtctx, logger, gatewayTarget, gateways, secrets, res); err != nil {
				gatewayClientCertErrors[gatewayTarget] = err
			}

			policy := &api.Policy{
				Key:    btls.Namespace + "/" + btls.Name + backendTlsPolicySuffix + attachmentName(policyTarget),
				Name:   TypedResourceName(wellknown.BackendTLSPolicyKind, btls),
				Target: policyTarget,
				Kind: &api.Policy_Backend{
					Backend: &api.BackendPolicySpec{
						Kind: &api.BackendPolicySpec_BackendTls{
							BackendTls: res,
						},
					},
				},
			}
			policies = append(policies, AgwPolicy{
				Gateway: new(gatewayTarget),
				Policy:  policy,
			})
		}
	}
	ancestorStatus := make([]gwv1.PolicyAncestorStatus, 0, uniqueGateways.Len())
	for _, g := range slices.SortBy(uniqueGateways.UnsortedList(), types.NamespacedName.String) {
		pr := gwv1.ParentReference{
			Group: new(gwv1.Group(gvk.KubernetesGateway.Group)),
			Kind:  new(gwv1.Kind(gvk.KubernetesGateway.Kind)),
			Name:  gwv1.ObjectName(g.Name),
		}
		gatewayConds := conds
		if err := gatewayClientCertErrors[g]; err != nil {
			gatewayConds = copyConditionMap(conds)
			gatewayConds[string(gwv1.PolicyConditionAccepted)].Error = err
		}
		ancestorStatus = append(ancestorStatus, SetAncestorStatus(pr, status, btls.Generation, gatewayConds, gwv1.GatewayController(controllerName)))
	}
	status.Ancestors = MergeAncestors(controllerName, status.Ancestors, ancestorStatus)
	return status, policies
}

// checkConflicted verifies if this target for this BackendTLSPolicy is conflicted.
// Conflicted means there is a different BackendTLSPolicy, with the same target, and the other one is a higher priority.
// Note: allMatches doesn't filter by sectionName, so we do that here.
func checkConflicted(
	btls *gwv1.BackendTLSPolicy,
	target gwv1.LocalPolicyTargetReferenceWithSectionName,
	allMatches []*gwv1.BackendTLSPolicy,
) error {
	for _, m := range allMatches {
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
		if policyselection.HasHigherPriority(m, btls) {
			return fmt.Errorf("policy %v matches the same target but with higher priority", m.Name)
		}
	}
	return nil
}

func targetEqual(a, b gwv1.LocalPolicyTargetReferenceWithSectionName) bool {
	return a.Group == b.Group &&
		a.Kind == b.Kind &&
		a.Name == b.Name &&
		ptr.Equal(a.SectionName, b.SectionName)
}

// a sentinel value to send to agentgateway to signal that it should reject TLS connects due to invalid config
var dummyCaCert = []byte("invalid")
var dummyClientCert = []byte("invalid")

func copyConditionMap(conds map[string]*Condition) map[string]*Condition {
	out := make(map[string]*Condition, len(conds))
	for k, v := range conds {
		copied := *v
		out[k] = &copied
	}
	return out
}

func invalidBackendClientCertificate(message string) *ConfigError {
	return &ConfigError{
		Reason:  string(gwv1.PolicyReasonInvalid),
		Message: message,
	}
}

func applyGatewayBackendClientCert(
	krtctx krt.HandlerContext,
	logger *slog.Logger,
	gatewayNN types.NamespacedName,
	gateways krt.Collection[*gwv1.Gateway],
	secrets krt.Collection[*corev1.Secret],
	res *api.BackendPolicySpec_BackendTLS,
) *ConfigError {
	gtw := ptr.Flatten(krt.FetchOne(krtctx, gateways, krt.FilterKey(gatewayNN.String())))
	if gtw == nil || gtw.Spec.TLS == nil || gtw.Spec.TLS.Backend == nil || gtw.Spec.TLS.Backend.ClientCertificateRef == nil {
		return nil
	}
	mtlsClientRef := gtw.Spec.TLS.Backend.ClientCertificateRef
	skip := false
	var configErr *ConfigError
	if mtlsClientRef.Namespace != nil {
		// TODO Implement this later
		logger.Warn("ignoring Gateway.spec.tls.backend; cross namespace not permitted")
		configErr = invalidBackendClientCertificate("Gateway.spec.tls.backend.clientCertificateRef cross namespace reference is not permitted")
		skip = true
	}
	if mtlsClientRef.Kind != nil && *mtlsClientRef.Kind != wellknown.SecretKind {
		logger.Warn("ignoring Gateway.spec.tls.backend; only Secret is allowed")
		configErr = invalidBackendClientCertificate("Gateway.spec.tls.backend.clientCertificateRef must refer to a Secret")
		skip = true
	}
	if mtlsClientRef.Group != nil && string(*mtlsClientRef.Group) != wellknown.SecretGVK.Group {
		logger.Warn("ignoring Gateway.spec.tls.backend; only core is allowed")
		configErr = invalidBackendClientCertificate("Gateway.spec.tls.backend.clientCertificateRef must use the core API group")
		skip = true
	}
	if !skip {
		nn := types.NamespacedName{
			Namespace: gtw.Namespace,
			Name:      string(mtlsClientRef.Name),
		}
		scrt := ptr.Flatten(krt.FetchOne(krtctx, secrets, krt.FilterObjectName(nn)))
		if scrt == nil {
			logger.Warn("ignoring Gateway.spec.tls.backend; secret not found")
			configErr = invalidBackendClientCertificate("Gateway.spec.tls.backend.clientCertificateRef Secret not found")
		} else {
			if _, err := ValidateTlsSecretData(nn.Name, nn.Namespace, scrt.Data); err != nil {
				logger.Warn("ignoring Gateway.spec.tls.backend; secret invalid")
				configErr = invalidBackendClientCertificate("Gateway.spec.tls.backend.clientCertificateRef Secret invalid: " + err.Error())
			} else {
				res.Cert = scrt.Data[corev1.TLSCertKey]
				res.Key = scrt.Data[corev1.TLSPrivateKeyKey]
			}
		}
	}
	if res.Cert == nil || res.Key == nil {
		res.Cert = dummyClientCert
		res.Key = dummyClientCert
	}
	return configErr
}

func getBackendTLSCACert(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	btls *gwv1.BackendTLSPolicy,
	conds map[string]*Condition,
) ([]byte, error) {
	validation := btls.Spec.Validation
	if wk := validation.WellKnownCACertificates; wk != nil {
		switch kind := *wk; kind {
		case gwv1.WellKnownCACertificatesSystem:
			return nil, nil

		default:
			conds[string(gwv1.PolicyConditionAccepted)].Error = &ConfigError{
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
			conds[string(gwv1.BackendTLSPolicyReasonResolvedRefs)].Error = &ConfigError{
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
			conds[string(gwv1.BackendTLSPolicyReasonResolvedRefs)].Error = &ConfigError{
				Reason:  string(gwv1.BackendTLSPolicyReasonInvalidCACertificateRef),
				Message: "Certificate reference not found",
			}
			return nil, fmt.Errorf("certificate reference not found: %v", ref)
		}
		caCert, err := GetCACertFromConfigMap(ptr.Flatten(cfgmap))
		if err != nil {
			conds[string(gwv1.BackendTLSPolicyReasonResolvedRefs)].Error = &ConfigError{
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
