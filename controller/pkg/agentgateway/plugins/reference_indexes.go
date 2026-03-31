package plugins

import (
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/util/sets"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type ReferenceTypes struct {
	PolicyTargets func(krtctx krt.HandlerContext, namespace string, name gwv1.ObjectName, gk schema.GroupKind, sectionName *gwv1.SectionName) (*api.PolicyTarget, bool)
	PolicyBackend func(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error)
	RouteBackend  func(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error)
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

func (e *BackendReferenceError) Error() string {
	return e.Message
}

func DefaultReferenceTypes(agw *AgwCollections) ReferenceTypes {
	return ReferenceTypes{
		// AgentgatewayPolicy targets
		PolicyTargets: func(krtctx krt.HandlerContext, namespace string, name gwv1.ObjectName, gk schema.GroupKind, sectionName *gwv1.SectionName) (*api.PolicyTarget, bool) {
			key := namespace + "/" + string(name)
			switch gk {
			case wellknown.GatewayGVK.GroupKind():
				return &api.PolicyTarget{
					Kind: utils.GatewayTarget(namespace, string(name), sectionName),
				}, ptr.Flatten(krt.FetchOne(krtctx, agw.Gateways, krt.FilterKey(key))) != nil
			case wellknown.HTTPRouteGVK.GroupKind():
				return &api.PolicyTarget{
					Kind: utils.RouteTarget(namespace, string(name), wellknown.HTTPRouteGVK.Kind, sectionName),
				}, ptr.Flatten(krt.FetchOne(krtctx, agw.HTTPRoutes, krt.FilterKey(key))) != nil
			case wellknown.GRPCRouteGVK.GroupKind():
				return &api.PolicyTarget{
					Kind: utils.RouteTarget(namespace, string(name), wellknown.GRPCRouteGVK.Kind, sectionName),
				}, ptr.Flatten(krt.FetchOne(krtctx, agw.GRPCRoutes, krt.FilterKey(key))) != nil
			case wellknown.AgentgatewayBackendGVK.GroupKind():
				return &api.PolicyTarget{
					Kind: utils.BackendTarget(namespace, string(name), sectionName),
				}, ptr.Flatten(krt.FetchOne(krtctx, agw.Backends, krt.FilterKey(key))) != nil
			case wellknown.ServiceGVK.GroupKind():
				return &api.PolicyTarget{
					Kind: utils.ServiceTarget(namespace, string(name), sectionName),
				}, ptr.Flatten(krt.FetchOne(krtctx, agw.Services, krt.FilterKey(key))) != nil
			}
			return nil, false
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
				svc := ptr.Flatten(krt.FetchOne(krtctx, agw.Services, krt.FilterKey(key)))
				if svc == nil {
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
				be := ptr.Flatten(krt.FetchOne(krtctx, agw.Backends, krt.FilterKey(key)))
				if be == nil {
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
		ref.Port = uint32(svc.Spec.TargetPorts[0].Number) //nolint:gosec // G115: validated 1-65535
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
		svc := ptr.Flatten(krt.FetchOne(krtctx, agw.Services, krt.FilterKey(key)))
		if svc == nil {
			return ref, &BackendReferenceError{
				Reason:  BackendReferenceErrorReasonBackendNotFound,
				Message: fmt.Sprintf("backend(%s) not found", kubeutils.GetServiceHostname(string(name), ns)),
			}
		}
	case wellknown.AgentgatewayBackendGVK.GroupKind():
		key := ns + "/" + string(name)
		ref.Kind = &api.BackendReference_Backend{Backend: key}
		// Populate resp now, so even if the service doesn't exist we can return a better error (Service not found vs invalid)
		be := ptr.Flatten(krt.FetchOne(krtctx, agw.Backends, krt.FilterKey(key)))
		if be == nil {
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

func (r RouteAttachment) Equals(other RouteAttachment) bool {
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

func (a PolicyAttachment) Equals(other PolicyAttachment) bool {
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
	// Gateway --> Gateway: trivial, no collection needed
	// ListenerSet --> Gateway: NOT present; ListenerSet attachment not implemented (but really should be!) in AgentgatewayPolicy anyways

	explicitReferences ReferenceTypes
}

func (p ReferenceIndex) LookupGatewaysForTarget(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	switch object.Kind {
	case wellknown.GatewayGVK.Kind:
		// Trivial case
		return sets.New(object.NamespacedName)
	case wellknown.HTTPRouteGVK.Kind, wellknown.GRPCRouteGVK.Kind, wellknown.TCPRouteGVK.Kind, wellknown.TLSRouteGVK.Kind:
		gateways := sets.New[types.NamespacedName]()
		for _, ancestor := range krt.FetchOne(ctx, p.attachments, krt.FilterKey(object.String())).Objects {
			gateways.Insert(ancestor.Gateway)
		}
		return gateways
	default:
		gateways := sets.New[types.NamespacedName]()
		for _, ancestor := range krt.FetchOne(ctx, p.Ancestors, krt.FilterKey(object.String())).Objects {
			gateways.Insert(ancestor.Gateway)
		}
		return gateways
	}
}

func (p ReferenceIndex) LookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName) sets.Set[types.NamespacedName] {
	return p.lookupGatewaysForBackend(ctx, object, map[string]struct{}{})
}

func (p ReferenceIndex) lookupGatewaysForBackend(ctx krt.HandlerContext, object utils.TypedNamespacedName, seen map[string]struct{}) sets.Set[types.NamespacedName] {
	key := object.String()
	if _, ok := seen[key]; ok {
		return sets.New[types.NamespacedName]()
	}
	seen[key] = struct{}{}

	base := p.LookupGatewaysForTarget(ctx, object)
	if p.PolicyAttachments == nil {
		return base
	}
	for _, pref := range krt.FetchOne(ctx, p.PolicyAttachments, krt.FilterKey(key)).Objects {
		base = base.Union(p.lookupGatewaysForBackend(ctx, pref.Target, seen))
	}
	return base
}

func (p ReferenceIndex) WithPolicyAttachments(references krt.IndexCollection[utils.TypedNamespacedName, *PolicyAttachment]) ReferenceIndex {
	p.PolicyAttachments = references
	return p
}

func (p ReferenceIndex) PolicyTarget(krtctx krt.HandlerContext, namespace string, name gwv1.ObjectName, gk schema.GroupKind, sectionName *gwv1.SectionName) (*api.PolicyTarget, bool) {
	return p.explicitReferences.PolicyTargets(krtctx, namespace, name, gk, sectionName)
}

func (p ReferenceIndex) PolicyBackend(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error) {
	return p.explicitReferences.PolicyBackend(krtctx, defaultNamespace, gk, name, namespace, port)
}

func (p ReferenceIndex) RouteBackend(krtctx krt.HandlerContext, defaultNamespace string, gk schema.GroupKind, name gwv1.ObjectName, namespace *gwv1.Namespace, port *gwv1.PortNumber) (*api.BackendReference, error) {
	return p.explicitReferences.RouteBackend(krtctx, defaultNamespace, gk, name, namespace, port)
}
