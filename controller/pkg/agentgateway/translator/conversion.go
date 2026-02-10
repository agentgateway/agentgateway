package translator

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"maps"
	"sort"
	"strconv"
	"strings"

	"github.com/agentgateway/agentgateway/go/api"
	"github.com/golang/protobuf/ptypes/duration"
	"istio.io/api/annotation"
	"istio.io/istio/pilot/pkg/model/kstatus"
	"istio.io/istio/pkg/config/host"
	"istio.io/istio/pkg/config/protocol"
	"istio.io/istio/pkg/config/schema/gvk"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/log"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	klabels "k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"
	gatewayx "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	v1alpha2 "github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/utils"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

// ConvertHTTPRouteToAgw converts a HTTPRouteRule to an agentgateway HTTPRoute
func ConvertHTTPRouteToAgw(ctx RouteContext, r gwv1.HTTPRouteRule,
	obj *gwv1.HTTPRoute, pos int, matchPos int,
) (*api.Route, *reporter.RouteCondition) {
	routeRuleKey := strconv.Itoa(pos) + "." + strconv.Itoa(matchPos)
	res := &api.Route{
		// unique for route rule
		Key:  utils.InternalRouteRuleKey(obj.Namespace, obj.Name, routeRuleKey),
		Name: utils.RouteName(wellknown.HTTPRouteKind, obj.Namespace, obj.Name, r.Name),
		// filled in later
		ListenerKey: "",
	}

	if err := processRouteMatches(&r, res); err != nil {
		return nil, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionFalse,
			Reason:  "InvalidMatch",
			Message: fmt.Sprintf("failed to process route matches: %v", err),
		}
	}

	policies, policiesErr := BuildAgwTrafficPolicyFilters(ctx, obj.Namespace, r.Filters)
	res.TrafficPolicies = policies

	if err := ApplyTimeouts(&r, res); err != nil {
		return nil, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionFalse,
			Reason:  "TranslationError",
			Message: fmt.Sprintf("failed to apply builtin route timeout: %v", err),
		}
	}
	if err := ApplyRetries(&r, res); err != nil {
		return nil, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionFalse,
			Reason:  "TranslationError",
			Message: fmt.Sprintf("failed to apply builtin route retries: %v", err),
		}
	}

	backends, backendErr, err := buildAgwHTTPDestination(ctx, r.BackendRefs, obj.Namespace)
	if err != nil {
		return nil, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionFalse,
			Reason:  "BackendError",
			Message: fmt.Sprintf("failed to build backend destination: %v", err),
		}
	}
	res.Backends = backends

	res.Hostnames = convertHostnames(obj.Spec.Hostnames)

	if policiesErr != nil && !isPolicyErrorCritical(policiesErr) {
		return nil, policiesErr
	}
	return res, backendErr
}

// Helper function to process route matches
func processRouteMatches(r *gwv1.HTTPRouteRule, res *api.Route) error {
	for _, match := range r.Matches {
		path, err := CreateAgwPathMatch(match)
		if err != nil {
			return fmt.Errorf("path match Error: %v", err)
		}

		headers, err := CreateAgwHeadersMatch(match)
		if err != nil {
			return fmt.Errorf("headers match Error: %v", err)
		}

		method, err := CreateAgwMethodMatch(match)
		if err != nil {
			return fmt.Errorf("method match Error: %v", err)
		}

		query, err := CreateAgwQueryMatch(match)
		if err != nil {
			return fmt.Errorf("query match Error: %v", err)
		}

		res.Matches = append(res.GetMatches(), &api.RouteMatch{
			Path:        path,
			Headers:     headers,
			Method:      method,
			QueryParams: query,
		})
	}
	return nil
}

// Helper function to convert hostnames
func convertHostnames(hostnames []gwv1.Hostname) []string {
	return slices.Map(hostnames, func(h gwv1.Hostname) string {
		return string(h)
	})
}

// Helper function to determine if filter error is critical
func isPolicyErrorCritical(filterError *reporter.RouteCondition) bool {
	criticalReasons := []gwv1.RouteConditionReason{
		"FilterNotSupported",
		"FilterConfigInvalid",
		// Add other critical filter error reasons as needed
	}

	return slices.Contains(criticalReasons, filterError.Reason)
}

// ConvertTCPRouteToAgw converts a TCPRouteRule to an agentgateway TCPRoute
func ConvertTCPRouteToAgw(ctx RouteContext, r gwv1a2.TCPRouteRule,
	obj *gwv1a2.TCPRoute, pos int,
) (*api.TCPRoute, *reporter.RouteCondition) {
	routeRuleKey := strconv.Itoa(pos)
	res := &api.TCPRoute{
		// unique for route rule
		Key:         utils.InternalRouteRuleKey(obj.Namespace, obj.Name, routeRuleKey),
		Name:        utils.RouteName(wellknown.TCPRouteKind, obj.Namespace, obj.Name, r.Name),
		ListenerKey: "",
	}

	// Build TCP destinations
	route, backendErr, err := buildAgwTCPDestination(ctx, r.BackendRefs, obj.Namespace)
	if err != nil {
		logger.Error("failed to translate tcp destination", "err", err)
		return nil, err
	}
	res.Backends = route

	return res, backendErr
}

// ConvertGRPCRouteToAgw converts a GRPCRouteRule to an agentgateway HTTPRoute
func ConvertGRPCRouteToAgw(ctx RouteContext, r gwv1.GRPCRouteRule,
	obj *gwv1.GRPCRoute, pos int,
) (*api.Route, *reporter.RouteCondition) {
	routeRuleKey := strconv.Itoa(pos)
	res := &api.Route{
		// unique for route rule
		Key:         utils.InternalRouteRuleKey(obj.Namespace, obj.Name, routeRuleKey),
		Name:        utils.RouteName(wellknown.GRPCRouteKind, obj.Namespace, obj.Name, r.Name),
		ListenerKey: "",
	}

	// Convert GRPC matches to Agw format
	for _, match := range r.Matches {
		headers, err := CreateAgwGRPCHeadersMatch(match)
		if err != nil {
			logger.Error("failed to translate grpc header match", "err", err, "route_name", obj.Name, "route_ns", obj.Namespace)
			return nil, err
		}
		// For GRPC, we don't have path match in the traditional sense, so we'll derive it from method
		var path *api.PathMatch
		if match.Method != nil {
			// Convert GRPC method to path for routing purposes
			if match.Method.Service != nil && match.Method.Method != nil {
				pathStr := fmt.Sprintf("/%s/%s", *match.Method.Service, *match.Method.Method)
				path = &api.PathMatch{Kind: &api.PathMatch_Exact{Exact: pathStr}}
			} else if match.Method.Service != nil {
				pathStr := fmt.Sprintf("/%s/", *match.Method.Service)
				path = &api.PathMatch{Kind: &api.PathMatch_Exact{Exact: pathStr}}
			} else if match.Method.Method != nil {
				// Convert wildcard to regex: "/*/{method}" becomes "/[^/]+/{method}"
				pathStr := fmt.Sprintf("/[^/]+/%s", *match.Method.Method)
				path = &api.PathMatch{Kind: &api.PathMatch_Regex{Regex: pathStr}}
			}
		}
		res.Matches = append(res.GetMatches(), &api.RouteMatch{
			Path:    path,
			Headers: headers,
			// note: the RouteMatch method field only applies for http methods
		})
	}
	if len(res.Matches) == 0 {
		// HTTPRoute defaults in the CRD itself, but GRPCRoute does not.
		// Agentgateway expects there to always be a match set.
		res.Matches = []*api.RouteMatch{{
			Path: &api.PathMatch{Kind: &api.PathMatch_PathPrefix{PathPrefix: "/"}},
		}}
	}

	policies, err := BuildAgwGRPCTrafficPolicies(ctx, obj.Namespace, r.Filters)
	if err != nil {
		logger.Error("failed to translate grpc filter", "err", err, "route_name", obj.Name, "route_ns", obj.Namespace)
		return nil, err
	}
	res.TrafficPolicies = policies

	route, backendErr, err := buildAgwGRPCDestination(ctx, r.BackendRefs, obj.Namespace)
	if err != nil {
		logger.Error("failed to translate grpc destination", "err", err, "route_name", obj.Name, "route_ns", obj.Namespace)
		return nil, err
	}
	res.Backends = route
	res.Hostnames = slices.Map(obj.Spec.Hostnames, func(e gwv1.Hostname) string {
		return string(e)
	})
	return res, backendErr
}

// ConvertTLSRouteToAgw converts a TLSRouteRule to an agentgateway TCPRoute
func ConvertTLSRouteToAgw(ctx RouteContext, r gwv1a2.TLSRouteRule,
	obj *gwv1a2.TLSRoute, pos int,
) (*api.TCPRoute, *reporter.RouteCondition) {
	routeRuleKey := strconv.Itoa(pos)
	res := &api.TCPRoute{
		// unique for route rule
		Key:         utils.InternalRouteRuleKey(obj.Namespace, obj.Name, routeRuleKey),
		Name:        utils.RouteName(wellknown.TLSRouteKind, obj.Namespace, obj.Name, r.Name),
		ListenerKey: "",
	}

	// Build TLS destinations
	route, backendErr, err := buildAgwTLSDestination(ctx, r.BackendRefs, obj.Namespace)
	if err != nil {
		logger.Error("failed to translate tls destination", "err", err, "route_name", obj.Name, "route_ns", obj.Namespace)
		return nil, err
	}
	res.Backends = route

	// TLS Routes have hostnames in the spec (unlike TCP Routes)
	res.Hostnames = slices.Map(obj.Spec.Hostnames, func(e gwv1.Hostname) string {
		return string(e)
	})

	return res, backendErr
}

func buildAgwTCPDestination(
	ctx RouteContext,
	forwardTo []gwv1.BackendRef,
	ns string,
) ([]*api.RouteBackend, *reporter.RouteCondition, *reporter.RouteCondition) {
	if forwardTo == nil {
		return nil, nil, nil
	}

	var invalidBackendErr *reporter.RouteCondition
	var res []*api.RouteBackend
	for _, fwd := range forwardTo {
		dst, err := buildAgwDestination(ctx, gwv1.HTTPBackendRef{
			BackendRef: fwd,
			Filters:    nil, // TCP Routes don't have per-backend filters?
		}, ns, wellknown.TCPRouteGVK, ctx.Backends)
		if err != nil {
			logger.Error("error building agent gateway destination", "error", err)
			if isInvalidBackend(err) {
				invalidBackendErr = err
				// keep going, we will gracefully drop invalid backends
			} else {
				return nil, nil, err
			}
		}
		res = append(res, dst)
	}
	return res, invalidBackendErr, nil
}

func buildAgwTLSDestination(
	ctx RouteContext,
	forwardTo []gwv1.BackendRef,
	ns string,
) ([]*api.RouteBackend, *reporter.RouteCondition, *reporter.RouteCondition) {
	if forwardTo == nil {
		return nil, nil, nil
	}

	var invalidBackendErr *reporter.RouteCondition
	var res []*api.RouteBackend
	for _, fwd := range forwardTo {
		dst, err := buildAgwDestination(ctx, gwv1.HTTPBackendRef{
			BackendRef: fwd,
			Filters:    nil, // TLS Routes don't have per-backend filters
		}, ns, wellknown.TLSRouteGVK, ctx.Backends)
		if err != nil {
			logger.Error("error building agent gateway destination", "error", err)
			if isInvalidBackend(err) {
				invalidBackendErr = err
				// keep going, we will gracefully drop invalid backends
			} else {
				return nil, nil, err
			}
		}
		res = append(res, dst)
	}
	return res, invalidBackendErr, nil
}

// terminalFilterCombinationError creates a standardized error message for when multiple terminal filters are used together
func terminalFilterCombinationError(existingFilter, newFilter string) string {
	return fmt.Sprintf("Cannot combine multiple terminal filters: %s and %s are mutually exclusive. Only one terminal filter is allowed per route rule.", existingFilter, newFilter)
}

// BuildAgwTrafficPolicyFilters builds a list of agentgateway TrafficPolicySpec from a list of k8s gateway api HTTPRoute filters
func BuildAgwTrafficPolicyFilters(
	ctx RouteContext,
	ns string,
	inputFilters []gwv1.HTTPRouteFilter,
) ([]*api.TrafficPolicySpec, *reporter.RouteCondition) {
	var policies []*api.TrafficPolicySpec
	var hasTerminalFilter bool
	var terminalFilterType string

	var policyError *reporter.RouteCondition
	// Collect multiples of same-type filters to merge
	var mergedReqHdr *api.HeaderModifier
	var mergedRespHdr *api.HeaderModifier
	var mergedMirror []*api.RequestMirrors_Mirror
	for _, filter := range inputFilters {
		switch filter.Type {
		case gwv1.HTTPRouteFilterRequestHeaderModifier:
			h := CreateAgwHeadersFilter(filter.RequestHeaderModifier)
			if h == nil {
				continue
			}
			mergedReqHdr = mergeHeaderModifiers(mergedReqHdr, h)
		case gwv1.HTTPRouteFilterResponseHeaderModifier:
			h := CreateAgwResponseHeadersFilter(filter.ResponseHeaderModifier)
			if h == nil {
				continue
			}
			mergedRespHdr = mergeHeaderModifiers(mergedRespHdr, h)
		case gwv1.HTTPRouteFilterRequestRedirect:
			if hasTerminalFilter {
				policyError = &reporter.RouteCondition{
					Type:    gwv1.RouteConditionAccepted,
					Status:  metav1.ConditionFalse,
					Reason:  gwv1.RouteReasonIncompatibleFilters,
					Message: terminalFilterCombinationError(terminalFilterType, "RequestRedirect"),
				}
				continue
			}
			h := CreateAgwRedirectFilter(filter.RequestRedirect)
			if h == nil {
				continue
			}
			policies = append(policies, &api.TrafficPolicySpec{Kind: &api.TrafficPolicySpec_RequestRedirect{RequestRedirect: h}})
			hasTerminalFilter = true
			terminalFilterType = "RequestRedirect"
		case gwv1.HTTPRouteFilterRequestMirror:
			h, err := CreateAgwMirrorFilter(ctx, filter.RequestMirror, ns, wellknown.HTTPRouteGVK)
			if err != nil {
				if policyError == nil {
					policyError = err
				}
			} else {
				mergedMirror = append(mergedMirror, h)
			}
		case gwv1.HTTPRouteFilterURLRewrite:
			h := CreateAgwRewriteFilter(filter.URLRewrite)
			if h == nil {
				continue
			}
			policies = append(policies, h)
		case gwv1.HTTPRouteFilterCORS:
			h := createAgwCorsFilter(filter.CORS)
			if h == nil {
				continue
			}
			policies = append(policies, h)
		case gwv1.HTTPRouteFilterExternalAuth:
			h, err := CreateAgwExternalAuthFilter(ctx, filter.ExternalAuth, ns, wellknown.HTTPRouteGVK)
			if err != nil {
				if policyError == nil {
					policyError = err
				}
			}
			policies = append(policies, h)
		case gwv1.HTTPRouteFilterExtensionRef:
			err := createAgwExtensionRefFilter(filter.ExtensionRef)
			if err != nil {
				if policyError == nil {
					policyError = err
				}
				continue
			}
		default:
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonIncompatibleFilters,
				Message: fmt.Sprintf("unsupported filter type %q", filter.Type),
			}
		}
	}
	// Append merged header modifiers at the end to avoid duplicates
	if mergedReqHdr != nil {
		policies = append(policies, &api.TrafficPolicySpec{Kind: &api.TrafficPolicySpec_RequestHeaderModifier{RequestHeaderModifier: mergedReqHdr}})
	}
	if mergedRespHdr != nil {
		policies = append(policies, &api.TrafficPolicySpec{Kind: &api.TrafficPolicySpec_ResponseHeaderModifier{ResponseHeaderModifier: mergedRespHdr}})
	}
	if mergedMirror != nil {
		policies = append(policies, &api.TrafficPolicySpec{Kind: &api.TrafficPolicySpec_RequestMirror{RequestMirror: &api.RequestMirrors{Mirrors: mergedMirror}}})
	}
	return policies, policyError
}

// BuildAgwBackendPolicyFilters builds a list of agentgateway BackendPolicySpec from a list of k8s gateway api HTTPRoute filters
func BuildAgwBackendPolicyFilters(
	ctx RouteContext,
	ns string,
	inputFilters []gwv1.HTTPRouteFilter,
) ([]*api.BackendPolicySpec, *reporter.RouteCondition) {
	var policies []*api.BackendPolicySpec
	var hasTerminalFilter bool
	var terminalFilterType string

	var policyError *reporter.RouteCondition
	// Collect multiples of same-type filters to merge
	var mergedReqHdr *api.HeaderModifier
	var mergedRespHdr *api.HeaderModifier
	var mergedMirror []*api.RequestMirrors_Mirror
	for _, filter := range inputFilters {
		switch filter.Type {
		case gwv1.HTTPRouteFilterRequestHeaderModifier:
			h := CreateAgwHeadersFilter(filter.RequestHeaderModifier)
			if h == nil {
				continue
			}
			mergedReqHdr = mergeHeaderModifiers(mergedReqHdr, h)
		case gwv1.HTTPRouteFilterResponseHeaderModifier:
			h := CreateAgwResponseHeadersFilter(filter.ResponseHeaderModifier)
			if h == nil {
				continue
			}
			mergedRespHdr = mergeHeaderModifiers(mergedRespHdr, h)
		case gwv1.HTTPRouteFilterRequestRedirect:
			if hasTerminalFilter {
				policyError = &reporter.RouteCondition{
					Type:    gwv1.RouteConditionAccepted,
					Status:  metav1.ConditionFalse,
					Reason:  gwv1.RouteReasonIncompatibleFilters,
					Message: terminalFilterCombinationError(terminalFilterType, "RequestRedirect"),
				}
				continue
			}
			h := CreateAgwRedirectFilter(filter.RequestRedirect)
			if h == nil {
				continue
			}
			policies = append(policies, &api.BackendPolicySpec{Kind: &api.BackendPolicySpec_RequestRedirect{RequestRedirect: h}})
			hasTerminalFilter = true
			terminalFilterType = "RequestRedirect"
		case gwv1.HTTPRouteFilterRequestMirror:
			h, err := CreateAgwMirrorFilter(ctx, filter.RequestMirror, ns, wellknown.HTTPRouteGVK)
			if err != nil {
				if policyError == nil {
					policyError = err
				}
			} else {
				mergedMirror = append(mergedMirror, h)
			}
		default:
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonIncompatibleFilters,
				Message: fmt.Sprintf("unsupported filter type %q", filter.Type),
			}
		}
	}
	// Append merged header modifiers at the end to avoid duplicates
	if mergedReqHdr != nil {
		policies = append(policies, &api.BackendPolicySpec{Kind: &api.BackendPolicySpec_RequestHeaderModifier{RequestHeaderModifier: mergedReqHdr}})
	}
	if mergedRespHdr != nil {
		policies = append(policies, &api.BackendPolicySpec{Kind: &api.BackendPolicySpec_ResponseHeaderModifier{ResponseHeaderModifier: mergedRespHdr}})
	}
	if mergedMirror != nil {
		policies = append(policies, &api.BackendPolicySpec{Kind: &api.BackendPolicySpec_RequestMirror{RequestMirror: &api.RequestMirrors{Mirrors: mergedMirror}}})
	}
	return policies, policyError
}

// mergeHeaderModifiers merges two api.HeaderModifier instances by concatenating their Add/Set/Remove lists.
// Later entries are applied after earlier ones by preserving order in the resulting slices.
func mergeHeaderModifiers(dst, src *api.HeaderModifier) *api.HeaderModifier {
	if src == nil {
		return dst
	}
	if dst == nil {
		// Create a copy of src to avoid mutating input
		out := &api.HeaderModifier{}
		if len(src.Add) > 0 {
			out.Add = append([]*api.Header{}, src.Add...)
		}
		if len(src.Set) > 0 {
			out.Set = append([]*api.Header{}, src.Set...)
		}
		if len(src.Remove) > 0 {
			out.Remove = append([]string{}, src.Remove...)
		}
		return out
	}
	if len(src.Add) > 0 {
		dst.Add = append(dst.Add, src.Add...)
	}
	if len(src.Set) > 0 {
		dst.Set = append(dst.Set, src.Set...)
	}
	if len(src.Remove) > 0 {
		dst.Remove = append(dst.Remove, src.Remove...)
	}
	return dst
}

func createAgwCorsFilter(cors *gwv1.HTTPCORSFilter) *api.TrafficPolicySpec {
	if cors == nil {
		return nil
	}
	return &api.TrafficPolicySpec{
		Kind: &api.TrafficPolicySpec_Cors{Cors: &api.CORS{
			AllowCredentials: ptr.OrEmpty(cors.AllowCredentials),
			AllowHeaders:     slices.Map(cors.AllowHeaders, func(h gwv1.HTTPHeaderName) string { return string(h) }),
			AllowMethods:     slices.Map(cors.AllowMethods, func(m gwv1.HTTPMethodWithWildcard) string { return string(m) }),
			AllowOrigins:     slices.Map(cors.AllowOrigins, func(o gwv1.CORSOrigin) string { return string(o) }),
			ExposeHeaders:    slices.Map(cors.ExposeHeaders, func(h gwv1.HTTPHeaderName) string { return string(h) }),
			MaxAge: &duration.Duration{
				Seconds: int64(cors.MaxAge),
			},
		}},
	}
}

func buildAgwHTTPDestination(
	ctx RouteContext,
	forwardTo []gwv1.HTTPBackendRef,
	ns string,
) ([]*api.RouteBackend, *reporter.RouteCondition, *reporter.RouteCondition) {
	if forwardTo == nil {
		return nil, nil, nil
	}

	var invalidBackendErr *reporter.RouteCondition
	var res []*api.RouteBackend
	for _, fwd := range forwardTo {
		dst, err := buildAgwDestination(ctx, fwd, ns, wellknown.HTTPRouteGVK, ctx.Backends)
		if err != nil {
			logger.Error("erroring building agent gateway destination", "error", err)
			if isInvalidBackend(err) {
				invalidBackendErr = err
				// keep going, we will gracefully drop invalid backends
			} else {
				return nil, nil, err
			}
		}
		if dst != nil {
			policies, err := BuildAgwBackendPolicyFilters(ctx, ns, fwd.Filters)
			if err != nil {
				return nil, nil, err
			}
			dst.BackendPolicies = policies
		}
		res = append(res, dst)
	}
	return res, invalidBackendErr, nil
}

func buildAgwDestination(
	ctx RouteContext,
	to gwv1.HTTPBackendRef,
	ns string,
	k schema.GroupVersionKind,
	backendCol krt.Collection[*v1alpha2.AgentgatewayBackend],
) (*api.RouteBackend, *reporter.RouteCondition) {
	ref := NormalizeReference(to.Group, to.Kind, wellknown.ServiceGVK)
	// check if the reference is allowed
	if toNs := to.Namespace; toNs != nil && string(*toNs) != ns {
		if !ctx.Grants.BackendAllowed(ctx.Krt, k, to.Name, *toNs, ns, ref) {
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionResolvedRefs,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonRefNotPermitted,
				Message: fmt.Sprintf("backendRef %v/%v not accessible to a %s in namespace %q (missing a ReferenceGrant?)", to.Name, *toNs, k.Kind, ns),
			}
		}
	}

	namespace := ns // use default
	if to.Namespace != nil {
		namespace = string(*to.Namespace)
	}
	var invalidBackendErr *reporter.RouteCondition
	var hostname string
	weight := int32(1) // default
	if to.Weight != nil {
		weight = *to.Weight
	}
	rb := &api.RouteBackend{
		Weight: weight,
	}
	var port *gwv1.PortNumber

	switch ref.GroupKind() {
	case wellknown.InferencePoolGVK.GroupKind():
		if strings.Contains(string(to.Name), ".") {
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: "service name invalid; the name of the Service must be used, not the hostname.",
			}
		}
		hostname = kubeutils.GetInferenceServiceHostname(string(to.Name), namespace)
		key := namespace + "/" + string(to.Name)
		svc := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.InferencePools, krt.FilterKey(key)))
		logger.Debug("found pull pool for service", "svc", svc, "key", key)
		if svc == nil {
			invalidBackendErr = &reporter.RouteCondition{
				Type:    gwv1.RouteConditionResolvedRefs,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonBackendNotFound,
				Message: fmt.Sprintf("backendRef %s  not found", key),
			}
		} else {
			rb.Backend = &api.BackendReference{
				Kind: &api.BackendReference_Service_{
					Service: &api.BackendReference_Service{
						Hostname:  hostname,
						Namespace: namespace,
					},
				},
				// InferencePool only supports single port
				Port: uint32(svc.Spec.TargetPorts[0].Number), //nolint:gosec // G115: InferencePool TargetPort is int32 with validation 1-65535, always safe
			}
		}
	case wellknown.HostnameGVK.GroupKind():
		// Hostname is an Istio-specific backend kind where the name is the literal hostname
		// Used for referencing services by their full hostname (e.g., from ServiceEntry)
		// The actual resolution to ServiceEntry happens via the BackendIndex alias mechanism
		port = to.Port
		if port == nil {
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: "port is required in backendRef for Hostname kind",
			}
		}
		if to.Namespace != nil {
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: "namespace may not be set with Hostname type",
			}
		}
		// Use the name directly as the hostname
		hostname = string(to.Name)
		// Note: Backend validation happens via BackendIndex which uses the Hostname->ServiceEntry alias
		// No need to explicitly check ServiceEntries here as the BackendIndex handles the resolution
		rb.Backend = &api.BackendReference{
			Kind: &api.BackendReference_Service_{
				Service: &api.BackendReference_Service{
					Hostname:  hostname,
					Namespace: namespace,
				},
			},
			Port: uint32(*port), //nolint:gosec // G115: Gateway API PortNumber is int32 with validation 1-65535, always safe
		}
	case wellknown.ServiceGVK.GroupKind():
		port = to.Port
		if strings.Contains(string(to.Name), ".") {
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: "service name invalid; the name of the Service must be used, not the hostname.",
			}
		}
		hostname = kubeutils.GetServiceHostname(string(to.Name), namespace)
		key := namespace + "/" + string(to.Name)
		svc := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Services, krt.FilterKey(key)))
		if svc == nil {
			invalidBackendErr = &reporter.RouteCondition{
				Type:    gwv1.RouteConditionResolvedRefs,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonBackendNotFound,
				Message: fmt.Sprintf("backend(%s) not found", hostname),
			}
		}
		// TODO: All kubernetes service types currently require a Port, so we do this for everything; consider making this per-type if we have future types
		// that do not require port.
		if port == nil {
			// "Port is required when the referent is a Kubernetes Service."
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: "port is required in backendRef",
			}
		}
		rb.Backend = &api.BackendReference{
			Kind: &api.BackendReference_Service_{
				Service: &api.BackendReference_Service{
					Hostname:  hostname,
					Namespace: namespace,
				},
			},
			Port: uint32(*port), //nolint:gosec // G115: Gateway API PortNumber is int32 with validation 1-65535, always safe
		}
	case wellknown.AgentgatewayBackendGVK.GroupKind():
		backendRefKey := ns + "/" + string(to.Name)
		fetchedKgwBackend := krt.FetchOne(ctx.Krt, backendCol, krt.FilterKey(backendRefKey))
		if fetchedKgwBackend == nil {
			logger.Error("failed to get kgateway Backend", "backend", backendRefKey)
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionResolvedRefs,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonBackendNotFound,
				Message: fmt.Sprintf("kgateway Backend not found: %s", backendRefKey),
			}
		}
		kgwBackend := *fetchedKgwBackend
		logger.Debug("successfully resolved kgateway Backend", "backend", kgwBackend.Name)
		rb.Backend = &api.BackendReference{
			Kind: &api.BackendReference_Backend{
				Backend: backendRefKey,
			},
		}
	default:
		return nil, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionResolvedRefs,
			Status:  metav1.ConditionFalse,
			Reason:  gwv1.RouteReasonInvalidKind,
			Message: fmt.Sprintf("referencing unsupported backendRef: group %q kind %q", ptr.OrEmpty(to.Group), ptr.OrEmpty(to.Kind)),
		}
	}
	return rb, invalidBackendErr
}

var knownReferences = sets.New(
	wellknown.GatewayGVK,
	wellknown.XListenerSetGVK,
	wellknown.ServiceGVK,
	wellknown.ServiceEntryGVK,
	wellknown.SecretGVK,
	wellknown.ConfigMapGVK,
)
var allowedParentReferences = sets.New(
	wellknown.GatewayGVK,
	wellknown.XListenerSetGVK,
	wellknown.ServiceGVK,
	wellknown.ServiceEntryGVK,
)

// NormalizeReference normalizes group and kind references to a standard GVK format.
// If group or kind are nil/empty, it uses the default GVK's group/kind.
// Empty group is treated as "core" API group.
func NormalizeReference(group *gwv1.Group, kind *gwv1.Kind, defaultGVK schema.GroupVersionKind) schema.GroupVersionKind {
	result := defaultGVK

	if kind != nil && *kind != "" {
		result.Kind = string(*kind)
	}

	if group != nil {
		groupStr := string(*group)
		if groupStr == "" {
			// Empty group means "core" API group
			result.Group = ""
		} else {
			result.Group = groupStr
		}
	}
	for wk := range knownReferences {
		if wk.Group == result.Group && wk.Kind == result.Kind {
			return wk
		}
	}

	return result
}

// ToInternalParentReference converts a gwv1.ParentReference to a ParentKey.
func ToInternalParentReference(p gwv1.ParentReference, localNamespace string) (ParentKey, error) {
	ref := NormalizeReference(p.Group, p.Kind, wellknown.GatewayGVK)
	if !allowedParentReferences.Contains(wellknown.GatewayGVK) {
		return ParentKey{}, fmt.Errorf("unsupported Parent: %v/%v", p.Group, p.Kind)
	}
	return ParentKey{
		Kind: ref,
		Name: string(p.Name),
		// Unset namespace means "same namespace"
		Namespace: defaultString(p.Namespace, localNamespace),
	}, nil
}

// ReferenceAllowed validates if a route can reference a specified parent based on rules like section, port, and hostnames.
// Returns a *ParentError if the reference violates any constraints or is disallowed.
// Returns nil if the reference is valid and permitted for the given route and ParentInfo.
func ReferenceAllowed(
	ctx RouteContext,
	parent *ParentInfo,
	routeKind schema.GroupVersionKind,
	parentRef ParentReference,
	hostnames []gwv1.Hostname,
	localNamespace string,
) *ParentError {
	if parentRef.Kind == wellknown.ServiceGVK {
		key := parentRef.Namespace + "/" + parentRef.Name
		svc := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Services, krt.FilterKey(key)))

		// check that the referenced svc exists
		if svc == nil {
			return &ParentError{
				Reason:  ParentErrorNotAccepted,
				Message: fmt.Sprintf("parent service: %q not found", parentRef.Name),
			}
		}
	} else if parentRef.Kind == wellknown.ServiceEntryGVK {
		// check that the referenced svc entry exists
		key := parentRef.Namespace + "/" + parentRef.Name
		svcEntry := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.ServiceEntries, krt.FilterKey(key)))
		if svcEntry == nil {
			return &ParentError{
				Reason:  ParentErrorNotAccepted,
				Message: fmt.Sprintf("parent service entry: %q not found", parentRef.Name),
			}
		}
	} else {
		// First, check section and port apply. This must come first
		if parentRef.Port != 0 && parentRef.Port != parent.Port {
			return &ParentError{
				Reason:  ParentErrorNotAccepted,
				Message: fmt.Sprintf("port %v not found", parentRef.Port),
			}
		}
		if len(parentRef.SectionName) > 0 && parentRef.SectionName != parent.SectionName {
			return &ParentError{
				Reason:  ParentErrorNotAccepted,
				Message: fmt.Sprintf("sectionName %q not found", parentRef.SectionName),
			}
		}

		// Next check the hostnames are a match. This is a bi-directional wildcard match. Only one route
		// hostname must match for it to be allowed (but the others will be filtered at runtime)
		// If either is empty its treated as a wildcard which always matches

		if len(hostnames) == 0 {
			hostnames = []gwv1.Hostname{"*"}
		}
		if len(parent.Hostnames) > 0 {
			matched := false
			hostMatched := false
		out:
			for _, routeHostname := range hostnames {
				for _, parentHostNamespace := range parent.Hostnames {
					var parentNamespace, parentHostname string
					if strings.Contains(parentHostNamespace, "/") {
						spl := strings.Split(parentHostNamespace, "/")
						parentNamespace, parentHostname = spl[0], spl[1]
					} else {
						parentNamespace, parentHostname = "*", parentHostNamespace
					}

					hostnameMatch := host.Name(parentHostname).Matches(host.Name(routeHostname))
					namespaceMatch := parentNamespace == "*" || parentNamespace == localNamespace

					hostMatched = hostMatched || hostnameMatch
					if hostnameMatch && namespaceMatch {
						matched = true
						break out
					}
				}
			}
			if !matched {
				if hostMatched {
					return &ParentError{
						Reason: ParentErrorNotAllowed,
						Message: fmt.Sprintf(
							"hostnames matched parent hostname %q, but namespace %q is not allowed by the parent",
							parent.OriginalHostname, localNamespace,
						),
					}
				}
				return &ParentError{
					Reason: ParentErrorNoHostname,
					Message: fmt.Sprintf(
						"no hostnames matched parent hostname %q",
						parent.OriginalHostname,
					),
				}
			}
		}
	}
	// Also make sure this route kind is allowed
	matched := false
	for _, ak := range parent.AllowedKinds {
		if string(ak.Kind) == routeKind.Kind && ptr.OrDefault((*string)(ak.Group), gvk.GatewayClass.Group) == routeKind.Group {
			matched = true
			break
		}
	}
	if !matched {
		return &ParentError{
			Reason:  ParentErrorNotAllowed,
			Message: fmt.Sprintf("kind %v is not allowed", routeKind),
		}
	}
	return nil
}

func extractParentReferenceInfo(ctx RouteContext, parents RouteParents, obj controllers.Object) []RouteParentReference {
	routeRefs, hostnames, kind := GetCommonRouteInfo(obj)
	localNamespace := obj.GetNamespace()
	var parentRefs []RouteParentReference
	for _, ref := range routeRefs {
		ir, err := ToInternalParentReference(ref, localNamespace)
		if err != nil {
			continue
		}
		pk := ParentReference{
			ParentKey:   ir,
			SectionName: ptr.OrEmpty(ref.SectionName),
			Port:        ptr.OrEmpty(ref.Port),
		}
		gk := ir
		currentParents := parents.Fetch(ctx.Krt, gk)
		appendParent := func(pr *ParentInfo, pk ParentReference) {
			bannedHostnames := sets.New[string]()
			for _, gw := range currentParents {
				if gw == pr {
					continue // do not ban ourself
				}
				if gw.Port != pr.Port {
					continue
				}
				if gw.Protocol != pr.Protocol {
					continue
				}
				bannedHostnames.Insert(gw.OriginalHostname)
			}
			deniedReason := ReferenceAllowed(ctx, pr, kind, pk, hostnames, localNamespace)

			rpi := RouteParentReference{
				ParentGateway:     pr.ParentGateway,
				InternalName:      pr.InternalName,
				InternalKind:      ir.Kind,
				Hostname:          pr.OriginalHostname,
				DeniedReason:      deniedReason,
				OriginalReference: ref,
				BannedHostnames:   bannedHostnames.Copy().Delete(pr.OriginalHostname),
				ParentKey:         ir,
				ParentSection:     pr.SectionName,
				Accepted:          deniedReason == nil,
			}
			parentRefs = append(parentRefs, rpi)
		}
		for _, gw := range currentParents {
			appendParent(gw, pk)
		}
	}
	// Ensure stable order
	slices.SortBy(parentRefs, func(a RouteParentReference) string {
		return ParentRefString(a.OriginalReference)
	})
	return parentRefs
}

// https://github.com/kubernetes-sigs/gateway-api/blob/cea484e38e078a2c1997d8c7a62f410a1540f519/apis/v1beta1/httproute_types.go#L207-L212
func isInvalidBackend(err *reporter.RouteCondition) bool {
	return err.Reason == gwv1.RouteReasonRefNotPermitted ||
		err.Reason == gwv1.RouteReasonBackendNotFound ||
		err.Reason == gwv1.RouteReasonInvalidKind
}

// ParentKey holds info about a parentRef (eg route binding to a Gateway). This is a mirror of
// gwv1.ParentReference in a form that can be stored in a map
type ParentKey struct {
	Kind schema.GroupVersionKind
	// Name is the original name of the resource (eg Kubernetes Gateway name)
	Name string
	// Namespace is the namespace of the resource
	Namespace string
}

func (p ParentKey) String() string {
	return p.Kind.String() + "/" + p.Namespace + "/" + p.Name
}

// ParentReference holds the parent key, section name and port for a parent reference.
type ParentReference struct {
	ParentKey

	SectionName gwv1.SectionName
	Port        gwv1.PortNumber
}

func (p ParentReference) String() string {
	return p.ParentKey.String() + "/" + string(p.SectionName) + "/" + fmt.Sprint(p.Port)
}

// ParentInfo holds info about a "Parent" - something that can be referenced as a ParentRef in the API.
// Today, this is just Gateway
type ParentInfo struct {
	ParentGateway types.NamespacedName
	// +krtEqualsTodo ensure gateway class changes trigger equality differences
	ParentGatewayClassName string
	// InternalName refers to the internal name we can reference it by. For example "my-ns/my-gateway"
	InternalName string
	// AllowedKinds indicates which kinds can be admitted by this Parent
	AllowedKinds []gwv1.RouteGroupKind
	// Hostnames is the hostnames that must be match to reference to the Parent. For gateway this is listener hostname
	// Format is ns/hostname
	Hostnames []string
	// OriginalHostname is the unprocessed form of Hostnames; how it appeared in users' config
	OriginalHostname string

	SectionName    gwv1.SectionName
	Port           gwv1.PortNumber
	Protocol       gwv1.ProtocolType
	TLSPassthrough bool
}

// RouteParentReference holds information about a route's parent reference
type RouteParentReference struct {
	// InternalName refers to the internal name of the parent we can reference it by. For example "my-ns/my-gateway"
	InternalName string
	// InternalKind is the Group/Kind of the Parent
	InternalKind schema.GroupVersionKind
	// DeniedReason, if present, indicates why the reference was not valid
	DeniedReason *ParentError
	// OriginalReference contains the original reference
	OriginalReference gwv1.ParentReference
	// Hostname is the hostname match of the Parent, if any
	Hostname        string
	BannedHostnames sets.Set[string]
	ParentKey       ParentKey
	ParentSection   gwv1.SectionName
	Accepted        bool
	ParentGateway   types.NamespacedName
}

// FilteredReferences filters out references that are not accepted by the Parent.
func FilteredReferences(parents []RouteParentReference) []RouteParentReference {
	ret := make([]RouteParentReference, 0, len(parents))
	for _, p := range parents {
		if p.DeniedReason != nil {
			// We should filter this out
			continue
		}
		ret = append(ret, p)
	}
	// To ensure deterministic order, sort them
	sort.Slice(ret, func(i, j int) bool {
		return ret[i].InternalName < ret[j].InternalName
	})
	return ret
}

// IsManaged checks if a Gateway is managed (ie we create the Deployment and Service) or unmanaged.
// This is based on the address field of the spec. If address is set with a Hostname type, it should point to an existing
// Service that handles the gateway traffic. If it is not set, or refers to only a single IP, we will consider it managed and provision the Service.
// If there is an IP, we will set the `loadBalancerIP` type.
// While there is no defined standard for this in the API yet, it is tracked in https://github.com/kubernetes-sigs/gateway-api/issues/892.
// So far, this mirrors how out of clusters work (address set means to use existing IP, unset means to provision one),
// and there has been growing consensus on this model for in cluster deployments.
//
// Currently, the supported options are:
// * 1 Hostname value. This can be short Service name ingress, or FQDN ingress.ns.svc.cluster.local, example.com. If its a non-k8s FQDN it is a ServiceEntry.
// * 1 IP address. This is managed, with IP explicit
// * Nothing. This is managed, with IP auto assigned
//
// Not supported:
// Multiple hostname/IP - It is feasible but preference is to create multiple Gateways. This would also break the 1:1 mapping of GW:Service
// Mixed hostname and IP - doesn't make sense; user should define the IP in service
// NamedAddress - Service has no concept of named address. For cloud's that have named addresses they can be configured by annotations,
//
//	which users can add to the Gateway.
//
// If manual deployments are disabled, IsManaged() always returns true.
func IsManaged(gw *gwv1.GatewaySpec) bool {
	//if !features.EnableGatewayAPIManualDeployment {
	//	return true
	//}
	if len(gw.Addresses) == 0 {
		return true
	}
	if len(gw.Addresses) > 1 {
		return false
	}
	if t := gw.Addresses[0].Type; t == nil || *t == gwv1.IPAddressType {
		return true
	}
	return false
}

func ExtractGatewayServices(kgw *gwv1.Gateway) ([]string, *reporter.RouteCondition) {
	if IsManaged(&kgw.Spec) {
		return []string{kubeutils.ServiceFQDN(kgw.ObjectMeta)}, nil
	}
	gatewayServices := []string{}
	skippedAddresses := []string{}
	for _, addr := range kgw.Spec.Addresses {
		if addr.Type != nil && *addr.Type != gwv1.HostnameAddressType {
			// We only support HostnameAddressType. Keep track of invalid ones so we can Report in status.
			skippedAddresses = append(skippedAddresses, addr.Value)
			continue
		}
		// TODO: For now we are using Addresses. There has been some discussion of allowing inline
		// parameters on the class field like a URL, in which case we will probably just use that. See
		// https://github.com/kubernetes-sigs/gateway-api/pull/614
		fqdn := addr.Value
		if !strings.Contains(fqdn, ".") {
			// Short name, expand it
			fqdn = kubeutils.ServiceFQDN(kgw.ObjectMeta)
		}
		gatewayServices = append(gatewayServices, fqdn)
	}
	if len(skippedAddresses) > 0 {
		// Give error but return services, this is a soft failure
		return gatewayServices, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionFalse,
			Reason:  gwv1.RouteReasonUnsupportedValue,
			Message: fmt.Sprintf("only Hostname is supported, ignoring %v", skippedAddresses),
		}
	}
	if _, f := kgw.Annotations[annotation.NetworkingServiceType.Name]; f {
		// Give error but return services, this is a soft failure
		// Remove entirely in 1.20
		return gatewayServices, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionFalse,
			Reason:  gwv1.RouteReasonUnsupportedValue,
			Message: fmt.Sprintf("annotation %v is deprecated, use Spec.Infrastructure.Routeability", annotation.NetworkingServiceType.Name),
		}
	}
	return gatewayServices, nil
}

// BuildListener translates a k8s Gateway to an internal listener representation.
func BuildListener(
	ctx krt.HandlerContext,
	secrets krt.Collection[*corev1.Secret],
	configMaps krt.Collection[*corev1.ConfigMap],
	grants ReferenceGrants,
	namespaces krt.Collection[*corev1.Namespace],
	obj controllers.Object,
	status []gwv1.ListenerStatus,
	gw gwv1.GatewaySpec,
	l gwv1.Listener,
	listenerIndex int,
	portErr error,
) ([]string, *TLSInfo, []gwv1.ListenerStatus, bool) {
	listenerConditions := map[string]*Condition{
		string(gwv1.ListenerConditionAccepted): {
			Reason:  string(gwv1.ListenerReasonAccepted),
			Message: "No errors found",
		},
		string(gwv1.ListenerConditionProgrammed): {
			Reason:  string(gwv1.ListenerReasonProgrammed),
			Message: "No errors found",
		},
		string(gwv1.ListenerConditionConflicted): {
			Reason:  string(gwv1.ListenerReasonNoConflicts),
			Message: "No errors found",
			Status:  kstatus.StatusFalse,
		},
		string(gwv1.ListenerConditionResolvedRefs): {
			Reason:  string(gwv1.ListenerReasonResolvedRefs),
			Message: "No errors found",
		},
	}

	ok := true
	gwTls := resolveGatewayTLS(l.Port, gw.TLS)
	tlsInfo, err := buildTLS(ctx, secrets, configMaps, grants, gwTls, l.TLS, obj)
	if err == nil && tlsInfo != nil {
		// If there were no other errors, also check the Key/Cert are actually valid
		err = validateTLS(tlsInfo)
	}
	if err != nil {
		listenerConditions[string(gwv1.ListenerConditionResolvedRefs)].Error = err
		listenerConditions[string(gwv1.GatewayConditionProgrammed)].Error = &ConfigError{
			Reason:  string(gwv1.GatewayReasonInvalid),
			Message: "Bad TLS configuration",
		}
		ok = false
	}
	if portErr != nil {
		listenerConditions[string(gwv1.ListenerConditionAccepted)].Error = &ConfigError{
			Reason:  string(gwv1.ListenerReasonUnsupportedProtocol),
			Message: portErr.Error(),
		}
		ok = false
	}

	hostnames := buildHostnameMatch(ctx, obj.GetNamespace(), namespaces, l)
	// TODO: do we need this?
	_, perr := listenerProtocolToAgw(l.Protocol)
	if perr != nil {
		listenerConditions[string(gwv1.ListenerConditionAccepted)].Error = &ConfigError{
			Reason:  string(gwv1.ListenerReasonUnsupportedProtocol),
			Message: perr.Error(),
		}
		ok = false
	}

	updatedStatus := reportListenerCondition(listenerIndex, l, obj, status, listenerConditions)
	return hostnames, tlsInfo, updatedStatus, ok
}

func resolveGatewayTLS(port gwv1.PortNumber, gw *gwv1.GatewayTLSConfig) *gwv1.TLSConfig {
	if gw == nil || gw.Frontend == nil {
		return nil
	}
	f := gw.Frontend
	pp := slices.FindFunc(f.PerPort, func(portConfig gwv1.TLSPortConfig) bool {
		return portConfig.Port == port
	})
	if pp != nil {
		return &pp.TLS
	}
	return &f.Default
}

var supportedProtocols = sets.New(
	gwv1.HTTPProtocolType,
	gwv1.HTTPSProtocolType,
	gwv1.TLSProtocolType,
	gwv1.TCPProtocolType,
	gwv1.ProtocolType(protocol.HBONE))

func listenerProtocolToAgw(p gwv1.ProtocolType) (string, error) {
	switch p {
	// Standard protocol types
	case gwv1.HTTPProtocolType:
		return string(p), nil
	case gwv1.HTTPSProtocolType:
		return string(p), nil
	case gwv1.TLSProtocolType, gwv1.TCPProtocolType:
		// TODO: check if TLS/TCP alpha features are supported
		return string(p), nil
	case gwv1.ProtocolType(protocol.HBONE):
		return string(p), nil
	}
	up := gwv1.ProtocolType(strings.ToUpper(string(p)))
	if supportedProtocols.Contains(up) {
		return "", fmt.Errorf("protocol %q is unsupported. hint: %q (uppercase) may be supported", p, up)
	}
	// Note: the k8s.UDPProtocolType is explicitly left to hit this path
	return "", fmt.Errorf("protocol %q is unsupported", p)
}

// dummyTls is a sentinel value to send to agentgateway to signal that it should reject TLS connects due to invalid config
var dummyTls = &TLSInfo{
	Cert: []byte("invalid"),
	Key:  []byte("invalid"),
}

func validateTLS(certInfo *TLSInfo) *ConfigError {
	if _, err := tls.X509KeyPair(certInfo.Cert, certInfo.Key); err != nil {
		return &ConfigError{
			Reason:  InvalidTLS,
			Message: fmt.Sprintf("invalid certificate reference, the certificate is malformed: %v", err),
		}
	}
	if certInfo.CaCert != nil {
		if !x509.NewCertPool().AppendCertsFromPEM(certInfo.Cert) {
			return &ConfigError{
				Reason:  InvalidTLS,
				Message: fmt.Sprintf("invalid CA certificate reference, the bundle is malformed"),
			}
		}
	}
	return nil
}

func buildTLS(
	ctx krt.HandlerContext,
	secrets krt.Collection[*corev1.Secret],
	configMaps krt.Collection[*corev1.ConfigMap],
	grants ReferenceGrants,
	gatewayTLS *gwv1.TLSConfig,
	tls *gwv1.ListenerTLSConfig,
	gw controllers.Object,
) (*TLSInfo, *ConfigError) {
	if tls == nil {
		return nil, nil
	}
	mode := gwv1.TLSModeTerminate
	if tls.Mode != nil {
		mode = *tls.Mode
	}
	namespace := gw.GetNamespace()
	switch mode {
	case gwv1.TLSModeTerminate:
		// Important: all failures MUST include dummyTls, as this is the signal to the dataplane to actually do TLS (but fail)
		if len(tls.CertificateRefs) != 1 {
			// This is required in the API, should be rejected in validation
			return dummyTls, &ConfigError{Reason: InvalidTLS, Message: "exactly 1 certificateRefs should be present for TLS termination"}
		}
		tlsRes, err := buildSecretReference(ctx, tls.CertificateRefs[0], gw, secrets)
		if err != nil {
			return dummyTls, err
		}
		// If we are going to send a cert, validate we can access it
		sameNamespace := tlsRes.Source.Namespace == namespace
		objectKind := GvkFromObject(gw)
		if !sameNamespace && !grants.SecretAllowed(ctx, objectKind, tlsRes.Source, namespace) {
			return dummyTls, &ConfigError{
				Reason: InvalidListenerRefNotPermitted,
				Message: fmt.Sprintf(
					"certificateRef %v/%v not accessible to a Gateway in namespace %q (missing a ReferenceGrant?)",
					tls.CertificateRefs[0].Name, tlsRes.Source.Namespace, namespace,
				),
			}
		}

		if gatewayTLS != nil && gatewayTLS.Validation != nil && len(gatewayTLS.Validation.CACertificateRefs) > 0 {
			// TODO: add 'Mode'
			if len(gatewayTLS.Validation.CACertificateRefs) > 1 {
				return dummyTls, &ConfigError{
					Reason:  InvalidTLS,
					Message: "only one caCertificateRef is supported",
				}
			}
			caCertRef := gatewayTLS.Validation.CACertificateRefs[0]
			cred, err := buildCaCertificateReference(ctx, caCertRef, gw, configMaps, secrets)
			if err != nil {
				return dummyTls, err
			}
			sameNamespace := cred.Source.Namespace == namespace
			isSecret := cred.Kind == wellknown.SecretGVK.Kind
			if isSecret && !sameNamespace && !grants.SecretAllowed(ctx, GvkFromObject(gw), cred.Source, namespace) {
				return dummyTls, &ConfigError{
					Reason: InvalidListenerRefNotPermitted,
					Message: fmt.Sprintf(
						"caCertificateRef %v/%v not accessible to a Gateway in namespace %q (missing a ReferenceGrant?)",
						cred.Source.Namespace, caCertRef.Name, namespace,
					),
				}
			}
			tlsRes.Info.CaCert = cred.Info.CaCert
		}
		return &tlsRes.Info, nil
	case gwv1.TLSModePassthrough:
		// Handled outside of this function. This only handles termination
		return nil, nil
	}
	return nil, nil
}

func buildCaCertificateReference(
	ctx krt.HandlerContext,
	ref gwv1.ObjectReference,
	gw controllers.Object,
	configMaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
) (*SecretReference, *ConfigError) {
	namespace := ptr.OrDefault((*string)(ref.Namespace), gw.GetNamespace())
	name := string(ref.Name)
	res := SecretReference{
		Source: types.NamespacedName{
			Namespace: namespace,
			Name:      name,
		},
		Info: TLSInfo{},
	}

	switch NormalizeReference(&ref.Group, &ref.Kind, schema.GroupVersionKind{}) {
	case wellknown.ConfigMapGVK:
		res.Kind = wellknown.ConfigMapGVK.Kind
		cm := ptr.Flatten(krt.FetchOne(ctx, configMaps, krt.FilterObjectName(res.Source)))
		if cm == nil {
			return nil, &ConfigError{
				Reason:  InvalidTLS,
				Message: fmt.Sprintf("invalid CA certificate reference, configmap %v not found", res.Source),
			}
		}
		certInfo, err := ExtractRootFromString(cm.Data)
		if err != nil {
			return nil, &ConfigError{
				Reason:  InvalidTLS,
				Message: fmt.Sprintf("invalid CA certificate reference %v, %v", plainObjectReferenceString(ref), err),
			}
		}
		res.Info.CaCert = certInfo.Cert
	case wellknown.SecretGVK:
		res.Kind = wellknown.SecretGVK.Kind
		scrt := ptr.Flatten(krt.FetchOne(ctx, secrets, krt.FilterObjectName(res.Source)))
		if scrt == nil {
			return nil, &ConfigError{
				Reason:  InvalidTLS,
				Message: fmt.Sprintf("invalid CA certificate reference, secret %v not found", res.Source),
			}
		}
		certInfo, err := ExtractRoot(scrt.Data)
		if err != nil {
			return nil, &ConfigError{
				Reason:  InvalidTLS,
				Message: fmt.Sprintf("invalid CA certificate reference %v, %v", plainObjectReferenceString(ref), err),
			}
		}
		res.Info.CaCert = certInfo.Cert
	default:
		return nil, &ConfigError{
			Reason:  InvalidTLS,
			Message: fmt.Sprintf("invalid CA certificate reference %v, only secret and configmap are allowed", plainObjectReferenceString(ref)),
		}
	}

	return &res, nil
}

type SecretReference struct {
	Source types.NamespacedName
	Kind   string
	Info   TLSInfo
}

func buildSecretReference(
	ctx krt.HandlerContext,
	ref gwv1.SecretObjectReference,
	gw controllers.Object,
	secrets krt.Collection[*corev1.Secret],
) (*SecretReference, *ConfigError) {
	if NormalizeReference(ref.Group, ref.Kind, wellknown.SecretGVK) != wellknown.SecretGVK {
		return nil, &ConfigError{Reason: InvalidTLS, Message: fmt.Sprintf("invalid certificate reference %v, only secret is allowed", objectReferenceString(ref))}
	}

	secret := types.NamespacedName{
		Name:      string(ref.Name),
		Namespace: ptr.OrDefault((*string)(ref.Namespace), gw.GetNamespace()),
	}

	scrt := ptr.Flatten(krt.FetchOne(ctx, secrets, krt.FilterObjectName(secret)))
	if scrt == nil {
		return nil, &ConfigError{
			Reason:  InvalidTLS,
			Message: fmt.Sprintf("invalid certificate reference %v, secret not found", objectReferenceString(ref)),
		}
	}
	certInfo, err := ExtractCertInfo(scrt)
	if err != nil {
		return nil, &ConfigError{
			Reason:  InvalidTLS,
			Message: fmt.Sprintf("invalid certificate reference %v, %v", objectReferenceString(ref), err),
		}
	}
	res := SecretReference{
		Source: secret,
		Kind:   wellknown.SecretGVK.Kind,
		Info: TLSInfo{
			Cert: certInfo.Cert,
			Key:  certInfo.Key},
	}
	return &res, nil
}

func plainObjectReferenceString(ref gwv1.ObjectReference) string {
	return fmt.Sprintf("%s/%s/%s.%s", ref.Group, ref.Kind, ref.Name, ptr.OrEmpty(ref.Namespace))
}

func objectReferenceString(ref gwv1.SecretObjectReference) string {
	return fmt.Sprintf("%s/%s/%s.%s",
		ptr.OrEmpty(ref.Group),
		ptr.OrEmpty(ref.Kind),
		ref.Name,
		ptr.OrEmpty(ref.Namespace))
}

// ParentRefString returns a string representation of a ParentRef.
func ParentRefString(ref gwv1.ParentReference) string {
	return fmt.Sprintf("%s/%s/%s/%s/%d.%s",
		ptr.OrEmpty(ref.Group),
		ptr.OrEmpty(ref.Kind),
		ref.Name,
		ptr.OrEmpty(ref.SectionName),
		ptr.OrEmpty(ref.Port),
		ptr.OrEmpty(ref.Namespace))
}

// buildHostnameMatch generates a Gateway.spec.servers.hosts section from a listener
func buildHostnameMatch(ctx krt.HandlerContext, localNamespace string, namespaces krt.Collection[*corev1.Namespace], l gwv1.Listener) []string {
	// We may allow all hostnames or a specific one
	hostname := "*"
	if l.Hostname != nil {
		hostname = string(*l.Hostname)
	}

	resp := []string{}
	for _, ns := range namespacesFromSelector(ctx, localNamespace, namespaces, l.AllowedRoutes) {
		// This check is necessary to prevent adding a hostname with an invalid empty namespace
		if len(ns) > 0 {
			resp = append(resp, fmt.Sprintf("%s/%s", ns, hostname))
		}
	}

	// If nothing matched use ~ namespace (match nothing). We need this since its illegal to have an
	// empty hostname list, but we still need the Gateway provisioned to ensure status is properly set and
	// SNI matches are established; we just don't want to actually match any routing rules (yet).
	if len(resp) == 0 {
		return []string{"~/" + hostname}
	}
	return resp
}

// namespacesFromSelector determines a list of allowed namespaces for a given AllowedRoutes
func namespacesFromSelector(ctx krt.HandlerContext, localNamespace string, namespaceCol krt.Collection[*corev1.Namespace], lr *gwv1.AllowedRoutes) []string {
	// Default is to allow only the same namespace
	if lr == nil || lr.Namespaces == nil || lr.Namespaces.From == nil || *lr.Namespaces.From == gwv1.NamespacesFromSame {
		return []string{localNamespace}
	}
	if *lr.Namespaces.From == gwv1.NamespacesFromAll {
		return []string{"*"}
	}

	if lr.Namespaces.Selector == nil {
		// Should never happen, invalid config
		return []string{"*"}
	}

	// gateway-api has selectors, but Istio Gateway just has a list of names. We will run the selector
	// against all namespaces and get a list of matching namespaces that can be converted into a list
	// Istio can handle.
	ls, err := metav1.LabelSelectorAsSelector(lr.Namespaces.Selector)
	if err != nil {
		return nil
	}
	namespaces := []string{}
	namespaceObjects := krt.Fetch(ctx, namespaceCol)
	for _, ns := range namespaceObjects {
		if ls.Matches(toNamespaceSet(ns.Name, ns.Labels)) {
			namespaces = append(namespaces, ns.Name)
		}
	}
	// Ensure stable order
	sort.Strings(namespaces)
	return namespaces
}

// NamespaceNameLabel represents that label added automatically to namespaces is newer Kubernetes clusters
const NamespaceNameLabel = "kubernetes.io/metadata.name"

// toNamespaceSet converts a set of namespace labels to a Set that can be used to select against.
func toNamespaceSet(name string, labels map[string]string) klabels.Set {
	// If namespace label is not set, implicitly insert it to support older Kubernetes versions
	if labels[NamespaceNameLabel] == name {
		// Already set, avoid copies
		return labels
	}
	// First we need a copy to not modify the underlying object
	ret := make(map[string]string, len(labels)+1)
	maps.Copy(ret, labels)
	ret[NamespaceNameLabel] = name
	return ret
}

func GetCommonRouteInfo(spec any) ([]gwv1.ParentReference, []gwv1.Hostname, schema.GroupVersionKind) {
	switch t := spec.(type) {
	case *gwv1a2.TCPRoute:
		return t.Spec.ParentRefs, nil, wellknown.TCPRouteGVK
	case *gwv1a2.TLSRoute:
		return t.Spec.ParentRefs, t.Spec.Hostnames, wellknown.TLSRouteGVK
	case *gwv1.HTTPRoute:
		return t.Spec.ParentRefs, t.Spec.Hostnames, wellknown.HTTPRouteGVK
	case *gwv1b1.HTTPRoute:
		return t.Spec.ParentRefs, t.Spec.Hostnames, wellknown.HTTPRouteGVK
	case *gwv1.GRPCRoute:
		return t.Spec.ParentRefs, t.Spec.Hostnames, wellknown.GRPCRouteGVK
	default:
		log.Fatalf("unknown type %T", t)
		return nil, nil, schema.GroupVersionKind{}
	}
}

func defaultString[T ~string](s *T, def string) string {
	if s == nil {
		return def
	}
	return string(*s)
}

func toRouteKind(g schema.GroupVersionKind) gwv1.RouteGroupKind {
	return gwv1.RouteGroupKind{Group: (*gwv1.Group)(&g.Group), Kind: gwv1.Kind(g.Kind)}
}

// createAgwExtensionRefFilter creates Agw filter from Gateway API ExtensionRef filter
func createAgwExtensionRefFilter(
	extensionRef *gwv1.LocalObjectReference,
) *reporter.RouteCondition {
	if extensionRef == nil {
		return nil
	}

	// TODO: support other types of extension refs (TrafficPolicySpec, etc.) https://github.com/kgateway-dev/kgateway/issues/12037

	// Unsupported ExtensionRef
	return &reporter.RouteCondition{
		Type:    gwv1.RouteConditionAccepted,
		Status:  metav1.ConditionFalse,
		Reason:  gwv1.RouteReasonIncompatibleFilters,
		Message: fmt.Sprintf("unsupported ExtensionRef: %s/%s", extensionRef.Group, extensionRef.Kind),
	}
}

func routeGroupKindEqual(rgk1, rgk2 gwv1.RouteGroupKind) bool {
	return rgk1.Kind == rgk2.Kind && getGroup(rgk1) == getGroup(rgk2)
}

func getGroup(rgk gwv1.RouteGroupKind) gwv1.Group {
	return ptr.OrDefault(rgk.Group, wellknown.GatewayGroup)
}

// We can use istio's once they bump to v1 GW API
func GvkFromObject(obj any) schema.GroupVersionKind {
	switch obj.(type) {
	case *gwv1.Gateway:
		return wellknown.GatewayGVK
	case *gatewayx.XListenerSet:
		return wellknown.XListenerSetGVK
	default:
		panic("Uknown GVK")
	}
}

const (
	// The ID/name for the certificate chain in kubernetes tls secret.
	TLSSecretCert = "tls.crt"
	// The ID/name for the k8sKey in kubernetes tls secret.
	TLSSecretKey = "tls.key"
	// The ID/name for the CA certificate in kubernetes tls secret
	TLSSecretCaCert = "ca.crt"
)

// ExtractRootFromString extracts the root certificate
func ExtractRootFromString(data map[string]string) (certInfo *CertInfo, err error) {
	conv := make(map[string][]byte, len(data))
	for k, v := range data {
		conv[k] = []byte(v)
	}
	return ExtractRoot(conv)
}

// ExtractRoot extracts the root certificate
func ExtractRoot(data map[string][]byte) (certInfo *CertInfo, err error) {
	ret := &CertInfo{}
	if hasValue(data, TLSSecretCaCert) {
		ret.Cert = data[TLSSecretCaCert]
		return ret, nil
	}
	// No cert found. Try to generate a helpful error message
	if hasKeys(data, TLSSecretCaCert) {
		return nil, fmt.Errorf("found key %q, but it was empty", TLSSecretCaCert)
	}
	found := truncatedKeysMessage(data)
	return nil, fmt.Errorf("found secret, but didn't have expected keys %s; found: %s",
		TLSSecretCaCert, found)
}

// CertInfo wraps a certificate, key, and oscp staple information.
type CertInfo struct {
	// The certificate chain
	Cert []byte
	// The private key
	Key []byte
}

func hasKeys(d map[string][]byte, keys ...string) bool {
	for _, k := range keys {
		_, f := d[k]
		if !f {
			return false
		}
	}
	return true
}

func hasValue(d map[string][]byte, keys ...string) bool {
	for _, k := range keys {
		v := d[k]
		if len(v) == 0 {
			return false
		}
	}
	return true
}

func truncatedKeysMessage(data map[string][]byte) string {
	keys := []string{}
	for k := range data {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	if len(keys) < 3 {
		return strings.Join(keys, ", ")
	}
	return fmt.Sprintf("%s, and %d more...", strings.Join(keys[:3], ", "), len(keys)-3)
}

// ExtractCertInfo extracts server key, certificate, and OCSP staple
func ExtractCertInfo(scrt *corev1.Secret) (certInfo *CertInfo, err error) {
	ret := &CertInfo{}
	if hasValue(scrt.Data, TLSSecretCert, TLSSecretKey) {
		ret.Cert = scrt.Data[TLSSecretCert]
		ret.Key = scrt.Data[TLSSecretKey]
		return ret, nil
	}
	// No cert found. Try to generate a helpful error message
	if hasKeys(scrt.Data, TLSSecretCert, TLSSecretKey) {
		return nil, fmt.Errorf("found keys %q and %q, but they were empty", TLSSecretCert, TLSSecretKey)
	}
	found := truncatedKeysMessage(scrt.Data)
	return nil, fmt.Errorf("found secret, but didn't have expected keys (%s and %s); found: %s",
		TLSSecretCert, TLSSecretKey, found)
}
