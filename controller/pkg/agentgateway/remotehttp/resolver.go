package remotehttp

import (
	"crypto/tls"
	"fmt"
	"net/url"
	"strconv"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/policyselection"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// ResolvedBackend is the normalized backend shape required by the remote HTTP
// resolver. Additional backend kinds can resolve to this type to reuse the
// built-in static endpoint, TLS, and tunnel handling.
type ResolvedBackend struct {
	Static   *agentgateway.StaticBackend
	Policies *agentgateway.BackendFull
}

// BackendResolver resolves a backend object into the normalized shape used by
// remote HTTP fetches. The bool return is false when the referenced backend does
// not exist.
type BackendResolver func(krt.HandlerContext, types.NamespacedName) (*ResolvedBackend, bool, error)

type Inputs struct {
	ConfigMaps       krt.Collection[*corev1.ConfigMap]
	Services         krt.Collection[*corev1.Service]
	Backends         krt.Collection[*agentgateway.AgentgatewayBackend]
	PolicySelector   policyselection.Selector
	BackendResolvers map[schema.GroupKind]BackendResolver
}

type ResolveInput struct {
	ParentName       string
	DefaultNamespace string
	BackendRef       *gwv1.BackendObjectReference
	URL              *string
	Path             string
	DefaultPort      string
}

type Resolver interface {
	Resolve(krtctx krt.HandlerContext, input ResolveInput) (*ResolvedTarget, error)
}

type defaultResolver struct {
	cfgmaps          krt.Collection[*corev1.ConfigMap]
	services         krt.Collection[*corev1.Service]
	backendResolvers map[schema.GroupKind]BackendResolver
	policySelector   policyselection.Selector
}

// ParseHTTPURL validates a policy backend URL and resolves its effective port.
func ParseHTTPURL(raw string) (*url.URL, uint32, error) {
	parsed, err := url.Parse(raw)
	if err != nil {
		return nil, 0, err
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return nil, 0, fmt.Errorf("unsupported URL scheme %q", parsed.Scheme)
	}
	if parsed.Hostname() == "" {
		return nil, 0, fmt.Errorf("url must include a host")
	}
	port := parsed.Port()
	if port == "" {
		if parsed.Scheme == "https" {
			port = "443"
		} else {
			port = "80"
		}
	}
	p, err := strconv.ParseUint(port, 10, 32)
	if err != nil || p == 0 || p > 65535 {
		return nil, 0, fmt.Errorf("invalid URL port %q", port)
	}
	return parsed, uint32(p), nil
}

func NewResolver(inputs Inputs) Resolver {
	backendResolvers := map[schema.GroupKind]BackendResolver{}
	if inputs.Backends != nil {
		backendResolvers[wellknown.AgentgatewayBackendGVK.GroupKind()] = AgentgatewayBackendResolver(inputs.Backends)
	}
	for gk, resolver := range inputs.BackendResolvers {
		if resolver != nil {
			backendResolvers[gk] = resolver
		}
	}
	return &defaultResolver{
		cfgmaps:          inputs.ConfigMaps,
		services:         inputs.Services,
		backendResolvers: backendResolvers,
		policySelector:   inputs.PolicySelector,
	}
}

// AgentgatewayBackendResolver adapts AgentgatewayBackend collections to the
// generic remote HTTP backend resolver interface.
func AgentgatewayBackendResolver(backends krt.Collection[*agentgateway.AgentgatewayBackend]) BackendResolver {
	return func(krtctx krt.HandlerContext, nn types.NamespacedName) (*ResolvedBackend, bool, error) {
		backend := ptr.Flatten(krt.FetchOne(krtctx, backends, krt.FilterObjectName(nn)))
		if backend == nil {
			return nil, false, nil
		}
		return &ResolvedBackend{
			Static:   backend.Spec.Static,
			Policies: backend.Spec.Policies,
		}, true, nil
	}
}

func (r *defaultResolver) Resolve(krtctx krt.HandlerContext, input ResolveInput) (*ResolvedTarget, error) {
	if input.URL != nil {
		parsed, _, err := ParseHTTPURL(*input.URL)
		if err != nil {
			return nil, err
		}
		target := FetchTarget{URL: parsed.String()}
		return &ResolvedTarget{
			Key:    target.Key(),
			Target: target,
		}, nil
	}
	if input.BackendRef == nil {
		return nil, fmt.Errorf("backendRef or url is required")
	}
	path := strings.TrimPrefix(input.Path, "/")
	resolved, err := r.resolveConnection(krtctx, input.ParentName, input.DefaultNamespace, *input.BackendRef, input.DefaultPort)
	if err != nil {
		return nil, err
	}

	target := FetchTarget{
		ProxyURL: resolved.proxyURL,
	}

	if resolved.proxyTLS != nil {
		target.ProxyTransport = TransportFingerprint{
			Verification: resolved.proxyTLS.verification,
			ServerName:   resolved.proxyTLS.serverName,
			CABundleHash: resolved.proxyTLS.caBundleHash,
			NextProtos:   append([]string(nil), resolved.proxyTLS.nextProtos...),
		}
	}

	var proxyTLSConfig *tls.Config
	if resolved.proxyTLS != nil {
		proxyTLSConfig = resolved.proxyTLS.tlsConfig
	}

	if resolved.tls == nil {
		target.URL = fmt.Sprintf("http://%s/%s", resolved.connectHost, path)
		return &ResolvedTarget{
			Key:            target.Key(),
			Target:         target,
			ProxyTLSConfig: proxyTLSConfig,
		}, nil
	}

	target.URL = fmt.Sprintf("https://%s/%s", resolved.connectHost, path)
	target.Transport = TransportFingerprint{
		Verification: resolved.tls.verification,
		ServerName:   resolved.tls.serverName,
		CABundleHash: resolved.tls.caBundleHash,
		NextProtos:   append([]string(nil), resolved.tls.nextProtos...),
	}

	return &ResolvedTarget{
		Key:            target.Key(),
		Target:         target,
		TLSConfig:      resolved.tls.tlsConfig,
		ProxyTLSConfig: proxyTLSConfig,
	}, nil
}
