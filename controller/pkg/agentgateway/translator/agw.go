package translator

import (
	"fmt"
	"strings"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
)

// CreateAgwMethodMatch creates an agw MethodMatch from a HTTPRouteMatch.
func CreateAgwMethodMatch(match gwv1.HTTPRouteMatch) (*api.MethodMatch, *reporter.RouteCondition) {
	if match.Method == nil {
		return nil, nil
	}
	return &api.MethodMatch{
		Exact: string(*match.Method),
	}, nil
}

// CreateAgwQueryMatch creates an agw QueryMatch from a HTTPRouteMatch.
func CreateAgwQueryMatch(match gwv1.HTTPRouteMatch) ([]*api.QueryMatch, *reporter.RouteCondition) {
	res := []*api.QueryMatch{}
	for _, header := range match.QueryParams {
		tp := gwv1.QueryParamMatchExact
		if header.Type != nil {
			tp = *header.Type
		}
		switch tp {
		case gwv1.QueryParamMatchExact:
			res = append(res, &api.QueryMatch{
				Name:  string(header.Name),
				Value: &api.QueryMatch_Exact{Exact: header.Value},
			})
		case gwv1.QueryParamMatchRegularExpression:
			res = append(res, &api.QueryMatch{
				Name:  string(header.Name),
				Value: &api.QueryMatch_Regex{Regex: header.Value},
			})
		default:
			// Should never happen, unless a new field is added
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: fmt.Sprintf("unknown type: %q is not supported QueryMatch type", tp)}
		}
	}
	if len(res) == 0 {
		return nil, nil
	}
	return res, nil
}

// CreateAgwPathMatch creates an agw PathMatch from a HTTPRouteMatch.
func CreateAgwPathMatch(match gwv1.HTTPRouteMatch) (*api.PathMatch, *reporter.RouteCondition) {
	if match.Path == nil {
		return nil, nil
	}
	tp := gwv1.PathMatchPathPrefix
	if match.Path.Type != nil {
		tp = *match.Path.Type
	}
	// Path value must start with "/". If empty/nil, coerce to "/".
	dest := "/"
	if match.Path.Value != nil && *match.Path.Value != "" {
		dest = *match.Path.Value
	}
	if !strings.HasPrefix(dest, "/") {
		dest = "/" + dest
	}
	switch tp {
	case gwv1.PathMatchPathPrefix:
		// Spec: trailing "/" is ignored in a prefix (except the root "/").
		if dest != "/" {
			dest = strings.TrimRight(dest, "/")
			if dest == "" {
				dest = "/"
			}
		}
		return &api.PathMatch{
			Kind: &api.PathMatch_PathPrefix{
				PathPrefix: dest,
			},
		}, nil

	case gwv1.PathMatchExact:
		// EXACT: do not normalize trailing slash; it must match byte-for-byte.
		return &api.PathMatch{
			Kind: &api.PathMatch_Exact{
				Exact: dest,
			},
		}, nil
	case gwv1.PathMatchRegularExpression:
		// Pass regex through unchanged.
		return &api.PathMatch{
			Kind: &api.PathMatch_Regex{
				Regex: dest,
			},
		}, nil
	default:
		// Defensive: unknown type => UnsupportedValue.
		return nil, &reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionFalse,
			Reason:  gwv1.RouteReasonUnsupportedValue,
			Message: fmt.Sprintf("unsupported Path match type %q", tp),
		}
	}
}

// CreateAgwHeadersMatch creates an agw HeadersMatch from a HTTPRouteMatch.
func CreateAgwHeadersMatch(match gwv1.HTTPRouteMatch) ([]*api.HeaderMatch, *reporter.RouteCondition) {
	var res []*api.HeaderMatch
	for _, header := range match.Headers {
		tp := gwv1.HeaderMatchExact
		if header.Type != nil {
			tp = *header.Type
		}
		switch tp {
		case gwv1.HeaderMatchExact:
			res = append(res, &api.HeaderMatch{
				Name:  string(header.Name),
				Value: &api.HeaderMatch_Exact{Exact: header.Value},
			})
		case gwv1.HeaderMatchRegularExpression:
			res = append(res, &api.HeaderMatch{
				Name:  string(header.Name),
				Value: &api.HeaderMatch_Regex{Regex: header.Value},
			})
		default:
			// Should never happen, unless a new field is added
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: fmt.Sprintf("unknown type: %q is not supported HeaderMatch type", tp)}
		}
	}

	if len(res) == 0 {
		return nil, nil
	}
	return res, nil
}

// CreateAgwHeadersFilter creates an agw HeaderModifier based on a HTTPHeaderFilter
func CreateAgwHeadersFilter(filter *gwv1.HTTPHeaderFilter) *api.HeaderModifier {
	if filter == nil {
		return nil
	}
	return &api.HeaderModifier{
		Add:    headerListToAgw(filter.Add),
		Set:    headerListToAgw(filter.Set),
		Remove: filter.Remove,
	}
}

// CreateAgwResponseHeadersFilter creates an agw TrafficPolicySpec based on a HTTPHeaderFilter
func CreateAgwResponseHeadersFilter(filter *gwv1.HTTPHeaderFilter) *api.HeaderModifier {
	if filter == nil {
		return nil
	}
	return &api.HeaderModifier{
		Add:    headerListToAgw(filter.Add),
		Set:    headerListToAgw(filter.Set),
		Remove: filter.Remove,
	}
}

// CreateAgwRewriteFilter creates an agw TrafficPolicySpec based on a HTTPURLRewriteFilter
func CreateAgwRewriteFilter(filter *gwv1.HTTPURLRewriteFilter) *api.TrafficPolicySpec {
	if filter == nil {
		return nil
	}

	var hostname string
	if filter.Hostname != nil {
		hostname = string(*filter.Hostname)
	}
	ff := &api.UrlRewrite{
		Host: hostname,
	}
	if filter.Path != nil {
		switch filter.Path.Type {
		case gwv1.PrefixMatchHTTPPathModifier:
			ff.Path = &api.UrlRewrite_Prefix{Prefix: strings.TrimSuffix(*filter.Path.ReplacePrefixMatch, "/")}
		case gwv1.FullPathHTTPPathModifier:
			ff.Path = &api.UrlRewrite_Full{Full: strings.TrimSuffix(*filter.Path.ReplaceFullPath, "/")}
		}
	}
	return &api.TrafficPolicySpec{
		Kind: &api.TrafficPolicySpec_UrlRewrite{
			UrlRewrite: ff,
		},
	}
}

// CreateAgwMirrorFilter creates an agw RequestMirror based on a HTTPRequestMirrorFilter
func CreateAgwMirrorFilter(
	ctx RouteContext,
	filter *gwv1.HTTPRequestMirrorFilter,
	ns string,
	k schema.GroupVersionKind,
) (*api.RequestMirrors_Mirror, *reporter.RouteCondition) {
	if filter == nil {
		return nil, nil
	}
	var weightOne int32 = 1
	dst, err := buildAgwDestination(ctx, gwv1.HTTPBackendRef{
		BackendRef: gwv1.BackendRef{
			BackendObjectReference: filter.BackendRef,
			Weight:                 &weightOne,
		},
	}, ns, k, ctx.Backends)
	if err != nil {
		return nil, err
	}
	var percent float64
	if f := filter.Fraction; f != nil {
		denominator := float64(100)
		if f.Denominator != nil {
			denominator = float64(*f.Denominator)
		}
		percent = (100 * float64(f.Numerator)) / denominator
	} else if p := filter.Percent; p != nil {
		percent = float64(*p)
	} else {
		percent = 100
	}
	if percent == 0 {
		return nil, nil
	}
	return &api.RequestMirrors_Mirror{
		Percentage: percent,
		Backend:    dst.GetBackend(),
	}, nil
}

// CreateAgwExternalAuthFilter creates Agw filter from Gateway API ExternalAuth filter
func CreateAgwExternalAuthFilter(
	ctx RouteContext,
	filter *gwv1.HTTPExternalAuthFilter,
	ns string,
	k schema.GroupVersionKind,
) (*api.TrafficPolicySpec, *reporter.RouteCondition) {
	if filter == nil {
		return nil, nil
	}
	dst, err := buildAgwDestination(ctx, gwv1.HTTPBackendRef{
		BackendRef: gwv1.BackendRef{
			BackendObjectReference: filter.BackendRef,
			Weight:                 ptr.Of(int32(1)),
		},
	}, ns, k, ctx.Backends)
	if err != nil {
		return nil, err
	}
	pol := &api.TrafficPolicySpec_ExternalAuth{
		Target: dst.GetBackend(),
	}
	if b := filter.ForwardBody; b != nil {
		pol.IncludeRequestBody = &api.TrafficPolicySpec_ExternalAuth_BodyOptions{
			MaxRequestBytes: uint32(b.MaxSize),
			// "Bodies over that size must fail processing"
			AllowPartialMessage: false,
			// TODO(https://github.com/kubernetes-sigs/gateway-api/issues/4198): do we want this?
			PackAsBytes: false,
		}
	}
	switch filter.ExternalAuthProtocol {
	case gwv1.HTTPRouteExternalAuthHTTPProtocol:
		http := ptr.OrEmpty(filter.HTTPAuthConfig)
		pp := &api.TrafficPolicySpec_ExternalAuth_HTTPProtocol{
			// Not supported in the API
			Redirect: nil,
			// Not supported in the API
			AddRequestHeaders: nil,
			// Not supported in the API
			Metadata: nil,
		}
		if http.Path != "" {
			path := http.Path
			if !strings.HasPrefix(path, "/") {
				path = "/" + path
			}
			pp.Path = ptr.Of(fmt.Sprintf("%q + request.path", path))
		}
		// Per spec, this must always be included
		pol.IncludeRequestHeaders = []string{
			"Authorization",
			":authority",
		}
		if len(http.AllowedRequestHeaders) > 0 {
			pol.IncludeRequestHeaders = append(pol.IncludeRequestHeaders, http.AllowedRequestHeaders...)
		}
		pp.IncludeResponseHeaders = http.AllowedResponseHeaders
		pol.Protocol = &api.TrafficPolicySpec_ExternalAuth_Http{
			Http: pp,
		}
	case gwv1.HTTPRouteExternalAuthGRPCProtocol:
		grpc := ptr.OrEmpty(filter.GRPCAuthConfig)
		pp := &api.TrafficPolicySpec_ExternalAuth_GRPCProtocol{
			// Not supported in the API
			Context: nil,
			// Not supported in the API
			Metadata: nil,
		}
		if len(grpc.AllowedRequestHeaders) > 0 {
			pol.IncludeRequestHeaders = grpc.AllowedRequestHeaders
		}
		pol.Protocol = &api.TrafficPolicySpec_ExternalAuth_Grpc{
			Grpc: pp,
		}
	}
	return &api.TrafficPolicySpec{
		Kind: &api.TrafficPolicySpec_ExtAuthz{
			ExtAuthz: pol,
		},
	}, nil
}

// CreateAgwRedirectFilter converts a HTTPRequestRedirectFilter into an api.RequestRedirect for request redirection.
func CreateAgwRedirectFilter(filter *gwv1.HTTPRequestRedirectFilter) *api.RequestRedirect {
	if filter == nil {
		return nil
	}
	var scheme, host string
	var port, statusCode uint32
	if filter.Scheme != nil {
		scheme = *filter.Scheme
	}
	if filter.Hostname != nil {
		host = string(*filter.Hostname)
	}
	if filter.Port != nil {
		port = uint32(*filter.Port) //nolint:gosec // G115: Gateway API PortNumber is int32 with validation 1-65535, always safe
	}
	if filter.StatusCode != nil {
		statusCode = uint32(*filter.StatusCode) //nolint:gosec // G115: HTTP status codes are always positive integers (100-599)
	}

	ff := &api.RequestRedirect{
		Scheme: scheme,
		Host:   host,
		Port:   port,
		Status: statusCode,
	}
	if filter.Path != nil {
		switch filter.Path.Type {
		case gwv1.PrefixMatchHTTPPathModifier:
			ff.Path = &api.RequestRedirect_Prefix{Prefix: strings.TrimSuffix(*filter.Path.ReplacePrefixMatch, "/")}
		case gwv1.FullPathHTTPPathModifier:
			ff.Path = &api.RequestRedirect_Full{Full: strings.TrimSuffix(*filter.Path.ReplaceFullPath, "/")}
		}
	}
	return ff
}

func headerListToAgw(hl []gwv1.HTTPHeader) []*api.Header {
	return slices.Map(hl, func(hl gwv1.HTTPHeader) *api.Header {
		return &api.Header{
			Name:  string(hl.Name),
			Value: hl.Value,
		}
	})
}

// CreateAgwGRPCHeadersMatch creates an agw HeaderMatch from a GRPCRouteMatch.
func CreateAgwGRPCHeadersMatch(match gwv1.GRPCRouteMatch) ([]*api.HeaderMatch, *reporter.RouteCondition) {
	var res []*api.HeaderMatch
	for _, header := range match.Headers {
		tp := gwv1.GRPCHeaderMatchExact
		if header.Type != nil {
			tp = *header.Type
		}
		switch tp {
		case gwv1.GRPCHeaderMatchExact:
			res = append(res, &api.HeaderMatch{
				Name:  string(header.Name),
				Value: &api.HeaderMatch_Exact{Exact: header.Value},
			})
		case gwv1.GRPCHeaderMatchRegularExpression:
			res = append(res, &api.HeaderMatch{
				Name:  string(header.Name),
				Value: &api.HeaderMatch_Regex{Regex: header.Value},
			})
		default:
			// Should never happen, unless a new field is added
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonUnsupportedValue,
				Message: fmt.Sprintf("unknown type: %q is not supported HeaderMatch type", tp)}
		}
	}

	if len(res) == 0 {
		return nil, nil
	}
	return res, nil
}

// BuildAgwGRPCTrafficPolicies constructs gRPC route filters for agent gateway based on the input filters and route context.
func BuildAgwGRPCTrafficPolicies(
	ctx RouteContext,
	ns string,
	inputFilters []gwv1.GRPCRouteFilter,
) ([]*api.TrafficPolicySpec, *reporter.RouteCondition) {
	var policies []*api.TrafficPolicySpec
	var mirrorBackendErr *reporter.RouteCondition
	// Collect multiples of same-type filters to merge
	var mergedReqHdr *api.HeaderModifier
	var mergedRespHdr *api.HeaderModifier
	var mergedMirror []*api.RequestMirrors_Mirror
	for _, filter := range inputFilters {
		switch filter.Type {
		case gwv1.GRPCRouteFilterRequestHeaderModifier:
			h := CreateAgwHeadersFilter(filter.RequestHeaderModifier)
			if h == nil {
				continue
			}
			mergedReqHdr = mergeHeaderModifiers(mergedReqHdr, h)
		case gwv1.GRPCRouteFilterResponseHeaderModifier:
			h := CreateAgwResponseHeadersFilter(filter.ResponseHeaderModifier)
			if h == nil {
				continue
			}
			mergedRespHdr = mergeHeaderModifiers(mergedRespHdr, h)
		case gwv1.GRPCRouteFilterRequestMirror:
			h, err := CreateAgwMirrorFilter(ctx, filter.RequestMirror, ns, schema.GroupVersionKind{
				Group:   "gateway.networking.k8s.io",
				Version: "v1",
				Kind:    "GRPCRoute",
			})
			if err != nil {
				mirrorBackendErr = err
			} else {
				mergedMirror = append(mergedMirror, h)
			}
		// TODO(npolshak): add ExtensionRef support for TrafficPolicy https://github.com/kgateway-dev/kgateway/issues/12037
		default:
			return nil, &reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.RouteReasonIncompatibleFilters,
				Message: fmt.Sprintf("unsupported filter type %q", filter.Type),
			}
		}
	}
	if mergedReqHdr != nil {
		policies = append(policies, &api.TrafficPolicySpec{Kind: &api.TrafficPolicySpec_RequestHeaderModifier{RequestHeaderModifier: mergedReqHdr}})
	}
	if mergedRespHdr != nil {
		policies = append(policies, &api.TrafficPolicySpec{Kind: &api.TrafficPolicySpec_ResponseHeaderModifier{ResponseHeaderModifier: mergedRespHdr}})
	}
	if mergedMirror != nil {
		policies = append(policies, &api.TrafficPolicySpec{Kind: &api.TrafficPolicySpec_RequestMirror{RequestMirror: &api.RequestMirrors{Mirrors: mergedMirror}}})
	}
	return policies, mirrorBackendErr
}

// BuildAgwGRPCBackendPolicies constructs gRPC route filters for agent gateway based on the input filters and route context.
func BuildAgwGRPCBackendPolicies(
	ctx RouteContext,
	ns string,
	inputFilters []gwv1.GRPCRouteFilter,
) ([]*api.BackendPolicySpec, *reporter.RouteCondition) {
	var policies []*api.BackendPolicySpec
	var mirrorBackendErr *reporter.RouteCondition
	// Collect multiples of same-type filters to merge
	var mergedReqHdr *api.HeaderModifier
	var mergedRespHdr *api.HeaderModifier
	var mergedMirror []*api.RequestMirrors_Mirror
	for _, filter := range inputFilters {
		switch filter.Type {
		case gwv1.GRPCRouteFilterRequestHeaderModifier:
			h := CreateAgwHeadersFilter(filter.RequestHeaderModifier)
			if h == nil {
				continue
			}
			mergedReqHdr = mergeHeaderModifiers(mergedReqHdr, h)
		case gwv1.GRPCRouteFilterResponseHeaderModifier:
			h := CreateAgwResponseHeadersFilter(filter.ResponseHeaderModifier)
			if h == nil {
				continue
			}
			mergedRespHdr = mergeHeaderModifiers(mergedRespHdr, h)
		case gwv1.GRPCRouteFilterRequestMirror:
			h, err := CreateAgwMirrorFilter(ctx, filter.RequestMirror, ns, schema.GroupVersionKind{
				Group:   "gateway.networking.k8s.io",
				Version: "v1",
				Kind:    "GRPCRoute",
			})
			if err != nil {
				mirrorBackendErr = err
			} else {
				mergedMirror = append(mergedMirror, h)
			}
		// TODO(npolshak): add ExtensionRef support for TrafficPolicy https://github.com/kgateway-dev/kgateway/issues/12037
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
	return policies, mirrorBackendErr
}

func buildAgwGRPCDestination(
	ctx RouteContext,
	forwardTo []gwv1.GRPCBackendRef,
	ns string,
) ([]*api.RouteBackend, *reporter.RouteCondition, *reporter.RouteCondition) {
	if forwardTo == nil {
		return nil, nil, nil
	}

	var invalidBackendErr *reporter.RouteCondition
	var res []*api.RouteBackend
	for _, fwd := range forwardTo {
		dst, err := buildAgwDestination(ctx, gwv1.HTTPBackendRef{
			BackendRef: fwd.BackendRef,
			Filters:    nil, // GRPC filters are handled separately
		}, ns, schema.GroupVersionKind{
			Group:   "gateway.networking.k8s.io",
			Version: "v1",
			Kind:    "GRPCRoute",
		}, ctx.Backends)
		if err != nil {
			logger.Error("error building agent gateway destination", "error", err)
			if isInvalidBackend(err) {
				invalidBackendErr = err
				// keep going, we will gracefully drop invalid backends
			} else {
				return nil, nil, err
			}
		}
		if dst != nil {
			policies, err := BuildAgwGRPCBackendPolicies(ctx, ns, fwd.Filters)
			if err != nil {
				return nil, nil, err
			}
			dst.BackendPolicies = policies
		}
		res = append(res, dst)
	}
	return res, invalidBackendErr, nil
}
