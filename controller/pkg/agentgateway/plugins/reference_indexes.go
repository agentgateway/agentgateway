package plugins

import (
	"fmt"
	"strconv"
	"strings"

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
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type ResolvedPolicySelectorTarget struct {
	Name          gwv1.ObjectName
	Namespace     string
	PolicyTargets []*api.PolicyTarget
	TargetErr     error
}

type ReferenceTypes struct {
	// KnownFromReferences is the set of GroupKinds this controller understands
	// as ReferenceGrant from entries.
	KnownFromReferences sets.Set[schema.GroupKind]
	// KnownToReferences is the set of GroupKinds this controller understands as
	// ReferenceGrant to entries. Extension authors must add their kinds here
	// when they need ReferenceGrant permission checks to recognize those targets.
	KnownToReferences       sets.Set[schema.GroupKind]
	PolicyTargets           func(krtctx krt.HandlerContext, namespace string, name gwv1.ObjectName, gk schema.GroupKind, sectionName *gwv1.SectionName, port *gwv1.PortNumber) ([]*api.PolicyTarget, error)
	PolicyTargetsBySelector func(krtctx krt.HandlerContext, policyNamespace string, selector agentgateway.LocalPolicyTargetSelectorWithSectionName) []ResolvedPolicySelectorTarget
	PolicyBackend           func(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error)
	RouteBackend            func(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error)
	AllowedParentReferences sets.Set[schema.GroupKind]
}

type BackendReferenceErrorReason string

const (
	BackendReferenceErrorReasonBackendNotFound  BackendReferenceErrorReason = "BackendNotFound"
	BackendReferenceErrorReasonInvalidKind      BackendReferenceErrorReason = "InvalidKind"
	BackendReferenceErrorReasonUnsupportedValue BackendReferenceErrorReason = "UnsupportedValue"
)

type BackendReferenceError struct {
	Reason  BackendReferenceErrorReason
	Message string
}

func ExtractName[T controllers.ComparableObject](t T) types.NamespacedName {
	return types.NamespacedName{
		Namespace: t.GetNamespace(),
		Name:      t.GetName(),
	}
}
func ResourceExists[T controllers.ComparableObject](krtctx krt.HandlerContext, col krt.Collection[T], key string) bool {
	return len(krt.PartialFetchComparable(krtctx, col, ExtractName, krt.FilterKey(key))) > 0
}

func (e *BackendReferenceError) Error() string {
	return e.Message
}

func DefaultReferenceTypes(agw *AgwCollections) ReferenceTypes {
	return ReferenceTypes{
		KnownFromReferences: sets.New(
			wellknown.AgentgatewayPolicyGVK.GroupKind(),
			// An AgentgatewayBackend is a grant source for its own backendRefs,
			// e.g. spec.policies.mcp.authentication.jwks.remote.
			wellknown.AgentgatewayBackendGVK.GroupKind(),
			wellknown.GatewayGVK.GroupKind(),
			wellknown.HTTPRouteGVK.GroupKind(),
			wellknown.GRPCRouteGVK.GroupKind(),
			wellknown.TCPRouteGVK.GroupKind(),
			wellknown.TLSRouteGVK.GroupKind(),
			wellknown.ListenerSetGVK.GroupKind(),
		),
		KnownToReferences: sets.New(
			wellknown.ServiceGVK.GroupKind(),
			wellknown.SecretGVK.GroupKind(),
			wellknown.AgentgatewayBackendGVK.GroupKind(),
		),
		// AgentgatewayPolicy targets
		PolicyTargets: func(krtctx krt.HandlerContext, namespace string, name gwv1.ObjectName, gk schema.GroupKind, sectionName *gwv1.SectionName, port *gwv1.PortNumber) ([]*api.PolicyTarget, error) {
			switch gk {
			case wellknown.GatewayGVK.GroupKind():
				return []*api.PolicyTarget{{
					Kind: utils.GatewayTarget(namespace, string(name), sectionName, port),
				}}, checkGatewayTarget(krtctx, agw.Gateways, namespace, string(name), sectionName, port)
			case wellknown.HTTPRouteGVK.GroupKind():
				return []*api.PolicyTarget{{
					Kind: utils.RouteTarget(namespace, string(name), wellknown.HTTPRouteGVK.Kind, sectionName),
				}}, checkSectionedTarget(krtctx, agw.HTTPRoutes, gk.Kind, namespace, string(name), sectionName, validateHTTPRouteRule)
			case wellknown.GRPCRouteGVK.GroupKind():
				return []*api.PolicyTarget{{
					Kind: utils.RouteTarget(namespace, string(name), wellknown.GRPCRouteGVK.Kind, sectionName),
				}}, checkSectionedTarget(krtctx, agw.GRPCRoutes, gk.Kind, namespace, string(name), sectionName, validateGRPCRouteRule)
			case wellknown.AgentgatewayBackendGVK.GroupKind():
				return []*api.PolicyTarget{{
					Kind: utils.BackendTarget(namespace, string(name), sectionName),
				}}, checkSectionedTarget(krtctx, agw.Backends, gk.Kind, namespace, string(name), sectionName, validateBackendSection)
			case wellknown.ServiceGVK.GroupKind():
				return []*api.PolicyTarget{{
					Kind: utils.ServiceTarget(namespace, string(name), sectionName),
				}}, checkSectionedTarget(krtctx, agw.Services, gk.Kind, namespace, string(name), sectionName, validateServicePort)
			case wellknown.ListenerSetGVK.GroupKind():
				return []*api.PolicyTarget{{
					Kind: utils.ListenerSetTarget(namespace, string(name), sectionName),
				}}, checkSectionedTarget(krtctx, agw.ListenerSets, gk.Kind, namespace, string(name), sectionName, validateListenerSetListener)
			case wellknown.InferencePoolGVK.GroupKind():
				hostname := kubeutils.GetInferenceServiceHostname(string(name), namespace)
				return []*api.PolicyTarget{{
					Kind: utils.ServiceTargetWithHostname(namespace, hostname, nil),
				}}, checkExists(krtctx, agw.InferencePools, gk.Kind, namespace, string(name))
			}
			return nil, fmt.Errorf("unsupported target kind %s", gk.Kind)
		},
		PolicyTargetsBySelector: func(krtctx krt.HandlerContext, policyNamespace string, selector agentgateway.LocalPolicyTargetSelectorWithSectionName) []ResolvedPolicySelectorTarget {
			targetGK := schema.GroupKind{Group: string(selector.Group), Kind: string(selector.Kind)}
			var targets []ResolvedPolicySelectorTarget
			sectionName := selector.SectionName

			switch targetGK {
			case wellknown.GatewayGVK.GroupKind():
				for _, gw := range krt.Fetch(krtctx, agw.Gateways, krt.FilterLabel(selector.MatchLabels), krt.FilterIndex(agw.GatewaysByNamespace, policyNamespace)) {
					policyTargets := []*api.PolicyTarget{{
						Kind: utils.GatewayTarget(gw.Namespace, gw.Name, sectionName, selector.Port),
					}}
					targets = append(targets, ResolvedPolicySelectorTarget{Name: gwv1.ObjectName(gw.Name), Namespace: gw.Namespace, PolicyTargets: policyTargets, TargetErr: validateGatewayListener(gw, sectionName, selector.Port)})
				}
			case wellknown.HTTPRouteGVK.GroupKind():
				for _, route := range krt.Fetch(krtctx, agw.HTTPRoutes, krt.FilterLabel(selector.MatchLabels), krt.FilterIndex(agw.HTTPRoutesByNamespace, policyNamespace)) {
					policyTargets := []*api.PolicyTarget{{
						Kind: utils.RouteTarget(route.Namespace, route.Name, wellknown.HTTPRouteGVK.Kind, sectionName),
					}}
					targets = append(targets, ResolvedPolicySelectorTarget{Name: gwv1.ObjectName(route.Name), Namespace: route.Namespace, PolicyTargets: policyTargets, TargetErr: validateHTTPRouteRule(route, sectionName)})
				}
			case wellknown.GRPCRouteGVK.GroupKind():
				for _, route := range krt.Fetch(krtctx, agw.GRPCRoutes, krt.FilterLabel(selector.MatchLabels), krt.FilterIndex(agw.GRPCRoutesByNamespace, policyNamespace)) {
					policyTargets := []*api.PolicyTarget{{
						Kind: utils.RouteTarget(route.Namespace, route.Name, wellknown.GRPCRouteGVK.Kind, sectionName),
					}}
					targets = append(targets, ResolvedPolicySelectorTarget{Name: gwv1.ObjectName(route.Name), Namespace: route.Namespace, PolicyTargets: policyTargets, TargetErr: validateGRPCRouteRule(route, sectionName)})
				}
			case wellknown.ListenerSetGVK.GroupKind():
				for _, ls := range krt.Fetch(krtctx, agw.ListenerSets, krt.FilterLabel(selector.MatchLabels), krt.FilterIndex(agw.ListenerSetsByNamespace, policyNamespace)) {
					policyTargets := []*api.PolicyTarget{{
						Kind: utils.ListenerSetTarget(ls.Namespace, ls.Name, sectionName),
					}}
					targets = append(targets, ResolvedPolicySelectorTarget{Name: gwv1.ObjectName(ls.Name), Namespace: ls.Namespace, PolicyTargets: policyTargets, TargetErr: validateListenerSetListener(ls, sectionName)})
				}
			case wellknown.AgentgatewayBackendGVK.GroupKind():
				for _, backend := range krt.Fetch(krtctx, agw.Backends, krt.FilterLabel(selector.MatchLabels), krt.FilterIndex(agw.BackendsByNamespace, policyNamespace)) {
					policyTargets := []*api.PolicyTarget{{
						Kind: utils.BackendTarget(backend.Namespace, backend.Name, sectionName),
					}}
					targets = append(targets, ResolvedPolicySelectorTarget{Name: gwv1.ObjectName(backend.Name), Namespace: backend.Namespace, PolicyTargets: policyTargets, TargetErr: validateBackendSection(backend, sectionName)})
				}
			case wellknown.ServiceGVK.GroupKind():
				for _, svc := range krt.Fetch(krtctx, agw.Services, krt.FilterLabel(selector.MatchLabels), krt.FilterIndex(agw.ServicesByNamespace, policyNamespace)) {
					policyTargets := []*api.PolicyTarget{{
						Kind: utils.ServiceTarget(svc.Namespace, svc.Name, sectionName),
					}}
					targets = append(targets, ResolvedPolicySelectorTarget{Name: gwv1.ObjectName(svc.Name), Namespace: svc.Namespace, PolicyTargets: policyTargets, TargetErr: validateServicePort(svc, sectionName)})
				}
			case wellknown.InferencePoolGVK.GroupKind():
				for _, pool := range krt.Fetch(krtctx, agw.InferencePools, krt.FilterLabel(selector.MatchLabels), krt.FilterIndex(agw.InferencePoolsByNamespace, policyNamespace)) {
					hostname := kubeutils.GetInferenceServiceHostname(pool.Name, pool.Namespace)
					policyTargets := []*api.PolicyTarget{{
						Kind: utils.ServiceTargetWithHostname(pool.Namespace, hostname, nil),
					}}
					targets = append(targets, ResolvedPolicySelectorTarget{Name: gwv1.ObjectName(pool.Name), Namespace: pool.Namespace, PolicyTargets: policyTargets})
				}
			}

			slices.SortFunc(targets, func(a, b ResolvedPolicySelectorTarget) int {
				if a.Namespace != b.Namespace {
					return strings.Compare(a.Namespace, b.Namespace)
				}
				return strings.Compare(string(a.Name), string(b.Name))
			})
			return targets
		},
		// AgentgatewayPolicy targets to backends (for things like ext_authz, etc)
		PolicyBackend: func(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error) {
			ns := string(ptr.OrDefault(namespace, gwv1.Namespace(defaultNamespace)))
			switch gk {
			case wellknown.ServiceGVK.GroupKind():
				if strings.Contains(string(name), ".") {
					return nil, &BackendReferenceError{
						Reason:  BackendReferenceErrorReasonUnsupportedValue,
						Message: "service name invalid; the name of the Service, not the hostname",
					}
				}
				key := ns + "/" + string(name)
				if !ResourceExists(krtctx, agw.Services, key) {
					return nil, &BackendReferenceError{
						Reason:  BackendReferenceErrorReasonBackendNotFound,
						Message: fmt.Sprintf("unable to find the Service %v", key),
					}
				}
				if port == nil {
					return nil, &BackendReferenceError{
						Reason:  BackendReferenceErrorReasonUnsupportedValue,
						Message: "port is required for Service targets",
					}
				}
				return &api.BackendReference{
					Kind: &api.BackendReference_Service_{
						Service: &api.BackendReference_Service{
							Hostname:  kubeutils.GetServiceHostname(string(name), ns),
							Namespace: ns,
						},
					},
					Port: uint32(*port), //nolint:gosec // G115: validated 1-65535
				}, nil
			case wellknown.AgentgatewayBackendGVK.GroupKind():
				key := ns + "/" + string(name)
				if !ResourceExists(krtctx, agw.Backends, key) {
					return nil, &BackendReferenceError{
						Reason:  BackendReferenceErrorReasonBackendNotFound,
						Message: fmt.Sprintf("unable to find the Backend %v", key),
					}
				}
				return &api.BackendReference{
					Kind: &api.BackendReference_Backend{
						Backend: key,
					},
				}, nil
			default:
				return nil, &BackendReferenceError{
					Reason:  BackendReferenceErrorReasonInvalidKind,
					Message: fmt.Sprintf("unsupported backend %v", gk),
				}
			}
		},
		RouteBackend: func(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error) {
			return DefaultRouteBackend(krtctx, agw, defaultNamespace, gk, name, namespace, port)
		},
		AllowedParentReferences: sets.New(
			wellknown.GatewayGVK.GroupKind(),
			wellknown.ListenerSetGVK.GroupKind(),
			wellknown.ServiceGVK.GroupKind(),
			wellknown.ServiceEntryGVK.GroupKind(),
		),
	}
}

func DefaultRouteBackend(krtctx krt.HandlerContext, agw *AgwCollections, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error) {
	ns := string(ptr.OrDefault(namespace, gwv1.Namespace(defaultNamespace)))
	// All MUST return a BackendReference. We may not be able to fully populate it, though; this will get replaced with 'invalid'
	ref := &api.BackendReference{}
	switch gk {
	case wellknown.InferencePoolGVK.GroupKind():
		if strings.Contains(string(name), ".") {
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonUnsupportedValue,
				Message: "InferencePool name invalid; the name of the InferencePool must be used, not the hostname.",
			}
		}
		key := ns + "/" + string(name)
		svc := ptr.Flatten(krt.FetchOne(krtctx, agw.InferencePools, krt.FilterKey(key)))
		if svc == nil {
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonBackendNotFound,
				Message: fmt.Sprintf("backendRef %s not found", key),
			}
		}
		ref.Kind = &api.BackendReference_Service_{
			Service: &api.BackendReference_Service{
				Hostname:  kubeutils.GetInferenceServiceHostname(string(name), ns),
				Namespace: ns,
			},
		}
		backendPort, err := utils.InferencePoolBackendPort(svc)
		if err != nil {
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonUnsupportedValue,
				Message: err.Error(),
			}
		}
		ref.Port = backendPort
	case wellknown.ServiceGVK.GroupKind():
		if strings.Contains(string(name), ".") {
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonUnsupportedValue,
				Message: "service name invalid; the name of the Service must be used, not the hostname.",
			}
		}
		if port == nil { // Validated by CEL so shouldn't happen
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonUnsupportedValue,
				Message: "port is required in Service backendRef",
			}
		}
		// Populate resp now, so even if the service doesn't exist we can return a better error (Service not found vs invalid)
		ref.Kind = &api.BackendReference_Service_{
			Service: &api.BackendReference_Service{
				Hostname:  kubeutils.GetServiceHostname(string(name), ns),
				Namespace: ns,
			}}
		ref.Port = uint32(*port) //nolint:gosec // G115: validated 1-65535
		key := ns + "/" + string(name)
		if !ResourceExists(krtctx, agw.Services, key) {
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonBackendNotFound,
				Message: fmt.Sprintf("backend(%s) not found", kubeutils.GetServiceHostname(string(name), ns)),
			}
		}
	case wellknown.AgentgatewayBackendGVK.GroupKind():
		key := ns + "/" + string(name)
		ref.Kind = &api.BackendReference_Backend{Backend: key}
		// Populate resp now, so even if the service doesn't exist we can return a better error (Service not found vs invalid)
		if !ResourceExists(krtctx, agw.Backends, key) {
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonBackendNotFound,
				Message: fmt.Sprintf("Backend not found: %s", key),
			}
		}
	default:
		return ref, &BackendReferenceError{
			Reason:  BackendReferenceErrorReasonInvalidKind,
			Message: fmt.Sprintf("referencing unsupported backendRef: group %q kind %q", gk.Group, gk.Kind),
		}
	}
	return ref, nil
}

type RouteAttachment struct {
	// Route
	From utils.TypedNamespacedName
	// Immediate parent (Gateway or ListenerSet)
	To           utils.TypedNamespacedName
	ListenerName string
	// Eventual parent (always Gateway)
	Gateway types.NamespacedName
}

func (r RouteAttachment) ResourceName() string {
	to := r.To.String()
	if r.To.Kind != wellknown.GatewayGVK.Kind {
		to += "/" + r.Gateway.String()
	}
	return r.From.Kind + "/" + r.From.NamespacedName.String() + "->" + to + "/" + r.ListenerName
}

func (r *RouteAttachment) Equals(other *RouteAttachment) bool {
	return r.From == other.From && r.To == other.To && r.ListenerName == other.ListenerName && r.Gateway == other.Gateway
}

// BuildReferenceIndex builds a set of indexes that can lookup objects through various means.
// For example, lookup associated Gateways for a Backend.
func BuildReferenceIndex(
	ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend],
	attachments krt.IndexCollection[utils.TypedNamespacedName, *RouteAttachment],
	referenceTypes ReferenceTypes,
) ReferenceIndex {
	return ReferenceIndex{
		Ancestors:          ancestors,
		attachments:        attachments,
		explicitReferences: referenceTypes,
	}
}

type PolicyAttachment struct {
	Target  utils.TypedNamespacedName
	Backend utils.TypedNamespacedName
	Source  utils.TypedNamespacedName
}

func (a *PolicyAttachment) Equals(other *PolicyAttachment) bool {
	return a.Target == other.Target && a.Backend == other.Backend && a.Source == other.Source
}

func (a PolicyAttachment) ResourceName() string {
	return a.Source.String() + "/" + a.Target.String() + "/" + a.Backend.String()
}

type ReferenceIndex struct {
	// Backend --> Gateway via Route
	Ancestors krt.IndexCollection[utils.TypedNamespacedName, *utils.AncestorBackend]
	// Backend --> Target via Policy
	PolicyAttachments krt.IndexCollection[utils.TypedNamespacedName, *PolicyAttachment]
	// Route --> Gateway
	attachments krt.IndexCollection[utils.TypedNamespacedName, *RouteAttachment]
	// ListenerSet --> Gateway
	listenerSetAttachments krt.IndexCollection[utils.TypedNamespacedName, *RouteAttachment]
	// Gateway --> Gateway: trivial, no collection needed

	explicitReferences ReferenceTypes
}

func (p ReferenceIndex) LookupGatewaysForTarget(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	switch object.Kind {
	case wellknown.GatewayGVK.Kind:
		// Trivial case
		return sets.New(object.NamespacedName)
	case wellknown.HTTPRouteGVK.Kind, wellknown.GRPCRouteGVK.Kind, wellknown.TCPRouteGVK.Kind, wellknown.TLSRouteGVK.Kind:
		gateways := sets.New[types.NamespacedName]()
		for _, ancestor := range krtutil.FetchIndexObjects(ctx, p.attachments, object) {
			gateways.Insert(ancestor.Gateway)
		}
		return gateways
	case wellknown.ListenerSetGVK.Kind:
		if p.listenerSetAttachments == nil {
			return sets.New[types.NamespacedName]()
		}
		gateways := sets.New[types.NamespacedName]()
		for _, attachment := range krtutil.FetchIndexObjects(ctx, p.listenerSetAttachments, object) {
			gateways.Insert(attachment.Gateway)
		}
		return gateways
	default:
		gateways := sets.New[types.NamespacedName]()
		for _, ancestor := range krtutil.FetchIndexObjects(ctx, p.Ancestors, object) {
			gateways.Insert(ancestor.Gateway)
		}
		return gateways
	}
}

func (p ReferenceIndex) LookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	return p.lookupGatewaysForBackend(ctx, object, sets.New[utils.TypedNamespacedName]())
}

func (p ReferenceIndex) lookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName, seen sets.Set[utils.TypedNamespacedName]) sets.Set[types.NamespacedName] {
	if seen.InsertContains(object) {
		return sets.New[types.NamespacedName]()
	}

	base := p.LookupGatewaysForTarget(ctx, object)
	if p.PolicyAttachments == nil {
		return base
	}
	for _, pref := range krtutil.FetchIndexObjects(ctx, p.PolicyAttachments, object) {
		base = base.Union(p.lookupGatewaysForBackend(ctx, pref.Target, seen))
	}
	return base
}

func IsBackendLikeTarget(policyTarget *api.PolicyTarget) bool {
	return policyTarget.GetBackend() != nil || policyTarget.GetService() != nil
}

// TODO: PolicyAttachment and AncestorBackend are keyed by root object only (no port/section).
// This means section-scoped targets (e.g. svc-a:9090 vs svc-a:8080) chain identically in the
// recursive graph, causing over-attachment. A follow-up should add section-aware indexing to
// support scoped recursive chaining without over-attachment.
func (p ReferenceIndex) LookupGatewaysForPolicyTarget(ctx krt.HandlerContext, object utils.TypedNamespacedName, policyTarget *api.PolicyTarget) sets.Set[types.NamespacedName] {
	if IsBackendLikeTarget(policyTarget) {
		return p.LookupGatewaysForBackend(ctx, object)
	}
	return p.LookupGatewaysForTarget(ctx, object)
}

func (p ReferenceIndex) WithPolicyAttachments(references krt.IndexCollection[utils.TypedNamespacedName, *PolicyAttachment]) ReferenceIndex {
	p.PolicyAttachments = references
	return p
}

func (p ReferenceIndex) WithListenerSetAttachments(references krt.IndexCollection[utils.TypedNamespacedName, *RouteAttachment]) ReferenceIndex {
	p.listenerSetAttachments = references
	return p
}

func (p ReferenceIndex) PolicyTarget(krtctx krt.HandlerContext, namespace string, name gwv1.ObjectName, gk schema.GroupKind, sectionName *gwv1.SectionName, port *gwv1.PortNumber) ([]*api.PolicyTarget, error) {
	return p.explicitReferences.PolicyTargets(krtctx, namespace, name, gk, sectionName, port)
}

func (p ReferenceIndex) PolicyTargetsBySelector(krtctx krt.HandlerContext, policyNamespace string, selector agentgateway.LocalPolicyTargetSelectorWithSectionName) []ResolvedPolicySelectorTarget {
	return p.explicitReferences.PolicyTargetsBySelector(krtctx, policyNamespace, selector)
}

func (p ReferenceIndex) PolicyBackend(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error) {
	return p.explicitReferences.PolicyBackend(krtctx, defaultNamespace, gk, name, namespace, port)
}

func (p ReferenceIndex) RouteBackend(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error) {
	return p.explicitReferences.RouteBackend(krtctx, defaultNamespace, gk, name, namespace, port)
}

func targetNotFound(kind, namespace, name string) error {
	return fmt.Errorf("%s %s/%s not found", kind, namespace, name)
}

func sectionNotFound[T controllers.ComparableObject](section gwv1.SectionName, kind string, obj T) error {
	return fmt.Errorf("sectionName %q not found in %s %s/%s", section, kind, obj.GetNamespace(), obj.GetName())
}

func checkExists[T controllers.ComparableObject](krtctx krt.HandlerContext, col krt.Collection[T], kind, namespace, name string) error {
	if !ResourceExists(krtctx, col, namespace+"/"+name) {
		return targetNotFound(kind, namespace, name)
	}
	return nil
}

// fetchTarget returns the named object, adding a krt dependency on the full object.
func fetchTarget[T controllers.ComparableObject](krtctx krt.HandlerContext, col krt.Collection[T], kind, namespace, name string) (T, error) {
	obj := krt.FetchOne(krtctx, col, krt.FilterKey(namespace+"/"+name))
	if obj == nil {
		var zero T
		return zero, targetNotFound(kind, namespace, name)
	}
	return *obj, nil
}

// checkSectionedTarget verifies a policy targetRef resolves. Targets scoped to a
// section fetch the full object so validate can check the section; unscoped targets
// only check existence to keep the krt dependency narrow.
func checkSectionedTarget[T controllers.ComparableObject](
	krtctx krt.HandlerContext,
	col krt.Collection[T],
	kind, namespace, name string,
	sectionName *gwv1.SectionName,
	validate func(obj T, sectionName *gwv1.SectionName) error,
) error {
	if sectionName == nil {
		return checkExists(krtctx, col, kind, namespace, name)
	}
	obj, err := fetchTarget(krtctx, col, kind, namespace, name)
	if err != nil {
		return err
	}
	return validate(obj, sectionName)
}

// checkGatewayTarget is the Gateway flavor of checkSectionedTarget; Gateways can also
// be scoped by listener port.
func checkGatewayTarget(krtctx krt.HandlerContext, gateways krt.Collection[*gwv1.Gateway], namespace, name string, sectionName *gwv1.SectionName, port *gwv1.PortNumber) error {
	if sectionName == nil && port == nil {
		return checkExists(krtctx, gateways, wellknown.GatewayGVK.Kind, namespace, name)
	}
	gw, err := fetchTarget(krtctx, gateways, wellknown.GatewayGVK.Kind, namespace, name)
	if err != nil {
		return err
	}
	return validateGatewayListener(gw, sectionName, port)
}

func validateGatewayListener(gw *gwv1.Gateway, sectionName *gwv1.SectionName, port *gwv1.PortNumber) error {
	if sectionName != nil {
		for _, l := range gw.Spec.Listeners {
			if l.Name == *sectionName {
				return nil
			}
		}
		return sectionNotFound(*sectionName, wellknown.GatewayGVK.Kind, gw)
	}
	if port != nil {
		for _, l := range gw.Spec.Listeners {
			if l.Port == *port {
				return nil
			}
		}
		return fmt.Errorf("port %d not found in %s %s/%s", *port, wellknown.GatewayGVK.Kind, gw.Namespace, gw.Name)
	}
	return nil
}

func validateListenerSetListener(ls *gwv1.ListenerSet, sectionName *gwv1.SectionName) error {
	if sectionName == nil {
		return nil
	}
	for _, l := range ls.Spec.Listeners {
		if l.Name == *sectionName {
			return nil
		}
	}
	return sectionNotFound(*sectionName, wellknown.ListenerSetGVK.Kind, ls)
}

func validateHTTPRouteRule(route *gwv1.HTTPRoute, sectionName *gwv1.SectionName) error {
	if sectionName == nil {
		return nil
	}
	for _, r := range route.Spec.Rules {
		if r.Name != nil && *r.Name == *sectionName {
			return nil
		}
	}
	return sectionNotFound(*sectionName, wellknown.HTTPRouteGVK.Kind, route)
}

func validateGRPCRouteRule(route *gwv1.GRPCRoute, sectionName *gwv1.SectionName) error {
	if sectionName == nil {
		return nil
	}
	for _, r := range route.Spec.Rules {
		if r.Name != nil && *r.Name == *sectionName {
			return nil
		}
	}
	return sectionNotFound(*sectionName, wellknown.GRPCRouteGVK.Kind, route)
}

// validateBackendSection checks for sub-backends (e.g. an MCP target or an LLM provider)
func validateBackendSection(be *agentgateway.AgentgatewayBackend, sectionName *gwv1.SectionName) error {
	if sectionName == nil {
		return nil
	}
	switch {
	case be.Spec.MCP != nil:
		for _, t := range be.Spec.MCP.Targets {
			if t.Name == *sectionName {
				return nil
			}
		}
	case be.Spec.AI != nil:
		if be.Spec.AI.LLM != nil && string(*sectionName) == utils.SingularLLMProviderSubBackendName {
			return nil
		}
		for _, g := range be.Spec.AI.PriorityGroups {
			for _, p := range g.Providers {
				if p.Name == *sectionName {
					return nil
				}
			}
		}
	}
	return sectionNotFound(*sectionName, wellknown.AgentgatewayBackendGVK.Kind, be)
}

// validateServicePort checks for a port number as the sectionName.
func validateServicePort(svc *corev1.Service, sectionName *gwv1.SectionName) error {
	if sectionName == nil {
		return nil
	}
	p, err := strconv.Atoi(string(*sectionName))
	if err != nil {
		return fmt.Errorf("sectionName %q is not a valid port number for %s %s/%s", *sectionName, wellknown.ServiceGVK.Kind, svc.Namespace, svc.Name)
	}
	for _, sp := range svc.Spec.Ports {
		if int(sp.Port) == p {
			return nil
		}
	}
	return fmt.Errorf("port %d not found in %s %s/%s", p, wellknown.ServiceGVK.Kind, svc.Namespace, svc.Name)
}
