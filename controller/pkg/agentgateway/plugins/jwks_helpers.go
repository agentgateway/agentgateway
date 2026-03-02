package plugins

import (
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks_url"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

const oidcDiscoveryTimeout = 10 * time.Second

var oidcResolverFactory = func() oidcResolver {
	return defaultOIDCResolver{}
}

var defaultOIDCDiscoveryClient = &http.Client{Timeout: oidcDiscoveryTimeout}

type oidcResolver interface {
	Resolve(ctx PolicyCtx, policyName, policyNamespace, issuer string, providerBackendRef *gwv1.BackendObjectReference) (*resolvedOIDCProvider, error)
}

type defaultOIDCResolver struct{}

type targetRefIndexKey struct {
	Name      string
	Kind      string
	Group     string
	Namespace string
}

func (k targetRefIndexKey) String() string {
	return fmt.Sprintf("%s/%s/%s/%s", k.Namespace, k.Group, k.Kind, k.Name)
}

type resolvedOIDCProvider struct {
	AuthorizationEndpoint             string
	TokenEndpoint                     string
	EndSessionEndpoint                string
	TokenEndpointAuthMethodsSupported []string
	JwksInline                        string
}

const oidcJWKSCacheTTL = 5 * time.Minute

type oidcDiscoveryDocument struct {
	AuthorizationEndpoint             string   `json:"authorization_endpoint"`
	TokenEndpoint                     string   `json:"token_endpoint"`
	JwksURI                           string   `json:"jwks_uri"`
	EndSessionEndpoint                string   `json:"end_session_endpoint"`
	TokenEndpointAuthMethodsSupported []string `json:"token_endpoint_auth_methods_supported"`
}

// resolveRemoteJWKSInline resolves a remote JWKS URI to an inline JWKS string by
// looking up the JWKS store ConfigMap and extracting the serialized JWKS JSON.
func resolveRemoteJWKSInline(ctx PolicyCtx, jwksURI string) (string, error) {
	if _, err := url.Parse(jwksURI); err != nil {
		return "", fmt.Errorf("invalid jwks url %w", err)
	}
	jwksStoreName := jwks.JwksConfigMapNamespacedName(jwksURI)
	if jwksStoreName == nil {
		return "", fmt.Errorf("jwks store hasn't been initialized")
	}
	jwksCM := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterObjectName(*jwksStoreName)))
	if jwksCM == nil {
		return "", fmt.Errorf("jwks ConfigMap %v isn't available", jwksStoreName)
	}
	jwksForURI, err := jwks.JwksFromConfigMap(jwksCM)
	if err != nil {
		return "", fmt.Errorf("error deserializing jwks ConfigMap %w", err)
	}
	inline, ok := jwksForURI[jwksURI]
	if !ok {
		return "", fmt.Errorf("jwks %s is not available in the jwks ConfigMap", jwksURI)
	}
	return inline, nil
}

func (defaultOIDCResolver) Resolve(
	ctx PolicyCtx,
	policyName, policyNamespace, issuer string,
	providerBackendRef *gwv1.BackendObjectReference,
) (*resolvedOIDCProvider, error) {
	metadata, err := fetchOIDCDiscoveryDocument(ctx, policyName, policyNamespace, issuer, providerBackendRef)
	if err != nil {
		return nil, err
	}
	if metadata.AuthorizationEndpoint == "" || metadata.TokenEndpoint == "" || metadata.JwksURI == "" {
		return nil, fmt.Errorf("oidc discovery document missing required endpoints for issuer %q", issuer)
	}
	jwksInline, err := resolveOIDCJWKSInline(ctx, policyName, policyNamespace, metadata.JwksURI, providerBackendRef)
	if err != nil {
		return nil, err
	}

	return &resolvedOIDCProvider{
		AuthorizationEndpoint:             metadata.AuthorizationEndpoint,
		TokenEndpoint:                     metadata.TokenEndpoint,
		EndSessionEndpoint:                metadata.EndSessionEndpoint,
		TokenEndpointAuthMethodsSupported: metadata.TokenEndpointAuthMethodsSupported,
		JwksInline:                        string(jwksInline),
	}, nil
}

func buildOIDCDiscoveryURL(issuer string) (string, error) {
	parsed, err := url.Parse(issuer)
	if err != nil {
		return "", fmt.Errorf("invalid issuer url %q: %w", issuer, err)
	}
	if parsed.Scheme == "" || parsed.Host == "" {
		return "", fmt.Errorf("issuer must be an absolute url: %q", issuer)
	}
	if parsed.RawQuery != "" || parsed.Fragment != "" {
		return "", fmt.Errorf("issuer must not contain query or fragment: %q", issuer)
	}
	if parsed.User != nil {
		return "", fmt.Errorf("issuer must not include userinfo: %q", issuer)
	}
	if parsed.Scheme != "https" {
		if parsed.Scheme != "http" || !isLoopbackHost(parsed.Hostname()) {
			return "", fmt.Errorf("issuer must use https (or http on loopback hosts): %q", issuer)
		}
	}
	parsed.Path = strings.TrimSuffix(parsed.Path, "/") + "/.well-known/openid-configuration"
	parsed.RawQuery = ""
	parsed.Fragment = ""
	return parsed.String(), nil
}

func isLoopbackHost(host string) bool {
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

func validateOIDCDiscoveryMetadataEndpoints(metadata *oidcDiscoveryDocument) error {
	validate := func(endpoint, field string) error {
		if endpoint == "" {
			return nil
		}
		parsed, err := url.Parse(endpoint)
		if err != nil {
			return fmt.Errorf("invalid %s URL in discovery metadata: %w", field, err)
		}
		if parsed.Fragment != "" || parsed.User != nil {
			return fmt.Errorf("%s must not contain fragment or userinfo", field)
		}
		if parsed.Host == "" {
			return fmt.Errorf("%s is missing host", field)
		}
		if parsed.Scheme != "https" {
			if parsed.Scheme != "http" || !isLoopbackHost(parsed.Hostname()) {
				return fmt.Errorf("%s must use https (or http on loopback hosts)", field)
			}
		}
		return nil
	}

	if err := validate(metadata.AuthorizationEndpoint, "authorization_endpoint"); err != nil {
		return err
	}
	if err := validate(metadata.TokenEndpoint, "token_endpoint"); err != nil {
		return err
	}
	if err := validate(metadata.JwksURI, "jwks_uri"); err != nil {
		return err
	}
	if err := validate(metadata.EndSessionEndpoint, "end_session_endpoint"); err != nil {
		return err
	}
	return nil
}

func fetchOIDCDiscoveryDocument(
	ctx PolicyCtx,
	policyName, policyNamespace, issuer string,
	providerBackendRef *gwv1.BackendObjectReference,
) (*oidcDiscoveryDocument, error) {
	discoveryURL, err := buildOIDCDiscoveryURL(issuer)
	if err != nil {
		return nil, err
	}
	fetcher := newProviderJSONFetcher(ctx, policyName, policyNamespace, providerBackendRef)
	var metadata oidcDiscoveryDocument
	if err := fetcher.fetch(discoveryURL, &metadata); err != nil {
		return nil, fmt.Errorf("failed fetching oidc discovery metadata: %w", err)
	}
	if err := validateOIDCDiscoveryMetadataEndpoints(&metadata); err != nil {
		return nil, fmt.Errorf("invalid oidc discovery metadata for issuer %q: %w", issuer, err)
	}
	return &metadata, nil
}

func resolveOIDCJWKSInline(
	ctx PolicyCtx,
	policyName, policyNamespace, jwksURI string,
	providerBackendRef *gwv1.BackendObjectReference,
) (string, error) {
	if inline, err := resolveRemoteJWKSInline(ctx, jwksURI); err == nil {
		return inline, nil
	}
	fetcher := newProviderJSONFetcher(ctx, policyName, policyNamespace, providerBackendRef)
	var jwks json.RawMessage
	if err := fetcher.fetch(jwksURI, &jwks); err != nil {
		return "", fmt.Errorf("failed fetching oidc jwks: %w", err)
	}
	jwksInline, err := json.Marshal(jwks)
	if err != nil {
		return "", fmt.Errorf("failed serializing oidc jwks: %w", err)
	}
	return string(jwksInline), nil
}

func ResolveOIDCJWKSSource(
	ctx PolicyCtx,
	policyName, policyNamespace, issuer string,
	providerBackendRef *gwv1.BackendObjectReference,
) (*jwks.JwksSource, error) {
	metadata, err := fetchOIDCDiscoveryDocument(ctx, policyName, policyNamespace, issuer, providerBackendRef)
	if err != nil {
		return nil, err
	}
	if metadata.JwksURI == "" {
		return nil, fmt.Errorf("oidc discovery document missing required jwks_uri for issuer %q", issuer)
	}
	jwksURL, hostOverride, tlsConfig, err := buildProviderFetchTarget(ctx, policyName, policyNamespace, providerBackendRef, metadata.JwksURI)
	if err != nil {
		return nil, err
	}
	return &jwks.JwksSource{
		JwksURL:      jwksURL,
		HostOverride: hostOverride,
		TlsConfig:    tlsConfig,
		Ttl:          oidcJWKSCacheTTL,
	}, nil
}

type providerJSONFetcher struct {
	ctx                PolicyCtx
	policyName         string
	policyNamespace    string
	providerBackendRef *gwv1.BackendObjectReference
	client             *http.Client
}

func newProviderJSONFetcher(
	ctx PolicyCtx,
	policyName, policyNamespace string,
	providerBackendRef *gwv1.BackendObjectReference,
) *providerJSONFetcher {
	return &providerJSONFetcher{
		ctx:                ctx,
		policyName:         policyName,
		policyNamespace:    policyNamespace,
		providerBackendRef: providerBackendRef,
		client:             defaultOIDCDiscoveryClient,
	}
}

func (f *providerJSONFetcher) fetch(target string, out any) error {
	requestURL, hostOverride, tlsConfig, err := buildProviderFetchTarget(
		f.ctx,
		f.policyName,
		f.policyNamespace,
		f.providerBackendRef,
		target,
	)
	if err != nil {
		return err
	}

	req, err := http.NewRequest(http.MethodGet, requestURL, nil)
	if err != nil {
		return fmt.Errorf("failed building request: %w", err)
	}
	if hostOverride != "" {
		req.Host = hostOverride
	}
	if tlsConfig != nil && f.client == defaultOIDCDiscoveryClient {
		f.client = &http.Client{
			Timeout: oidcDiscoveryTimeout,
			Transport: &http.Transport{
				TLSClientConfig: tlsConfig,
			},
		}
	}
	resp, err := f.client.Do(req)
	if err != nil {
		return fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		return fmt.Errorf("request failed with status %d: %s", resp.StatusCode, strings.TrimSpace(string(body)))
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 2<<20))
	if err != nil {
		return fmt.Errorf("failed reading response body: %w", err)
	}
	if err := json.Unmarshal(body, out); err != nil {
		return fmt.Errorf("failed decoding response body: %w", err)
	}
	return nil
}

func buildProviderFetchTarget(
	ctx PolicyCtx,
	policyName, policyNamespace string,
	providerBackendRef *gwv1.BackendObjectReference,
	target string,
) (string, string, *tls.Config, error) {
	targetURL, err := url.Parse(target)
	if err != nil {
		return "", "", nil, fmt.Errorf("invalid provider url %q: %w", target, err)
	}
	if providerBackendRef == nil {
		return targetURL.String(), "", nil, nil
	}
	requestURL, tlsConfig, err := buildProviderRequestURLAndTLS(
		ctx,
		policyName,
		policyNamespace,
		*providerBackendRef,
		targetURL,
	)
	if err != nil {
		return "", "", nil, err
	}
	return requestURL, targetURL.Host, tlsConfig, nil
}

func buildProviderRequestURLAndTLS(
	ctx PolicyCtx,
	policyName, policyNamespace string,
	backendRef gwv1.BackendObjectReference,
	targetURL *url.URL,
) (string, *tls.Config, error) {
	kind := ptr.OrDefault(backendRef.Kind, wellknown.ServiceKind)
	group := ptr.OrDefault(backendRef.Group, "")
	refNamespace := string(ptr.OrDefault(backendRef.Namespace, gwv1.Namespace(policyNamespace)))

	var (
		connectHost string
		tlsConfig   *tls.Config
	)

	switch {
	case string(kind) == wellknown.AgentgatewayBackendGVK.Kind && string(group) == wellknown.AgentgatewayBackendGVK.Group:
		backendNN := types.NamespacedName{Name: string(backendRef.Name), Namespace: refNamespace}
		backend := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.Backends, krt.FilterObjectName(backendNN)))
		if backend == nil {
			return "", nil, fmt.Errorf("backend %s not found, policy %s", backendNN, types.NamespacedName{Namespace: policyNamespace, Name: policyName})
		}
		if backend.Spec.Static == nil {
			return "", nil, fmt.Errorf("only static backends are supported for oidc provider backend; backend: %s, policy: %s", backendNN, types.NamespacedName{Namespace: policyNamespace, Name: policyName})
		}
		if backend.Spec.Policies != nil && backend.Spec.Policies.TLS != nil {
			var err error
			tlsConfig, err = jwks_url.GetTLSConfig(ctx.Krt, ctx.Collections.ConfigMaps, refNamespace, backend.Spec.Policies.TLS)
			if err != nil {
				return "", nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w", backendNN, types.NamespacedName{Namespace: policyNamespace, Name: policyName}, err)
			}
		} else if agwPolicy := targetedAgentgatewayPolicy(ctx, refNamespace, string(group), string(kind), string(backendRef.Name)); agwPolicy != nil && agwPolicy.Spec.Backend != nil && agwPolicy.Spec.Backend.TLS != nil {
			var err error
			tlsConfig, err = jwks_url.GetTLSConfig(ctx.Krt, ctx.Collections.ConfigMaps, refNamespace, agwPolicy.Spec.Backend.TLS)
			if err != nil {
				return "", nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w", backendNN, types.NamespacedName{Namespace: policyNamespace, Name: policyName}, err)
			}
		} else if backendTLSPolicy := targetedBackendTLSPolicy(ctx, refNamespace, string(kind), string(backendRef.Name)); backendTLSPolicy != nil {
			var err error
			tlsConfig, err = tlsConfigFromBackendTLSPolicy(ctx, refNamespace, backendTLSPolicy)
			if err != nil {
				return "", nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w", backendNN, types.NamespacedName{Namespace: policyNamespace, Name: policyName}, err)
			}
		}
		connectHost = fmt.Sprintf("%s:%d", backend.Spec.Static.Host, backend.Spec.Static.Port)
	case string(kind) == wellknown.ServiceKind && string(group) == "":
		if agwPolicy := targetedAgentgatewayPolicy(ctx, refNamespace, string(group), string(kind), string(backendRef.Name)); agwPolicy != nil && agwPolicy.Spec.Backend != nil && agwPolicy.Spec.Backend.TLS != nil {
			var err error
			tlsConfig, err = jwks_url.GetTLSConfig(ctx.Krt, ctx.Collections.ConfigMaps, refNamespace, agwPolicy.Spec.Backend.TLS)
			if err != nil {
				return "", nil, fmt.Errorf("error setting tls options; service %s/%s, policy: %s, %w", backendRef.Name, refNamespace, types.NamespacedName{Namespace: policyNamespace, Name: policyName}, err)
			}
		} else if backendTLSPolicy := targetedBackendTLSPolicy(ctx, refNamespace, string(kind), string(backendRef.Name)); backendTLSPolicy != nil {
			var err error
			tlsConfig, err = tlsConfigFromBackendTLSPolicy(ctx, refNamespace, backendTLSPolicy)
			if err != nil {
				return "", nil, fmt.Errorf("error setting tls options; service %s/%s, policy: %s, %w", backendRef.Name, refNamespace, types.NamespacedName{Namespace: policyNamespace, Name: policyName}, err)
			}
		}
		serviceHost := kubeutils.GetServiceHostname(string(backendRef.Name), refNamespace)
		port := ptr.OrEmpty(backendRef.Port)
		if port == 0 {
			if targetURL.Port() != "" {
				connectHost = fmt.Sprintf("%s:%s", serviceHost, targetURL.Port())
			} else {
				connectHost = serviceHost
			}
		} else {
			connectHost = fmt.Sprintf("%s:%d", serviceHost, port)
		}
	default:
		return "", nil, fmt.Errorf("unsupported oidc provider backend kind %s.%s for policy %s", group, kind, types.NamespacedName{Namespace: policyNamespace, Name: policyName})
	}

	scheme := targetURL.Scheme
	if tlsConfig != nil {
		scheme = "https"
		clone := tlsConfig.Clone()
		if clone.ServerName == "" {
			clone.ServerName = targetURL.Hostname()
		}
		tlsConfig = clone
	} else if strings.EqualFold(targetURL.Scheme, "https") {
		tlsConfig = &tls.Config{
			MinVersion: tls.VersionTLS12,
			ServerName: targetURL.Hostname(),
		}
		scheme = "https"
	}

	requestURL := &url.URL{
		Scheme:   scheme,
		Host:     connectHost,
		Path:     targetURL.EscapedPath(),
		RawPath:  targetURL.RawPath,
		RawQuery: targetURL.RawQuery,
	}
	return requestURL.String(), tlsConfig, nil
}

func targetedAgentgatewayPolicy(ctx PolicyCtx, namespace, group, kind, name string) *agentgateway.AgentgatewayPolicy {
	return ptr.Flatten(krt.FetchOne(
		ctx.Krt,
		ctx.Collections.AgentgatewayPolicies,
		krt.FilterIndex(ctx.Collections.PoliciesByTargetRef, targetRefIndexKey{
			Name:      name,
			Kind:      kind,
			Group:     group,
			Namespace: namespace,
		}),
	))
}

func targetedBackendTLSPolicy(ctx PolicyCtx, namespace, kind, name string) *gwv1.BackendTLSPolicy {
	return ptr.Flatten(krt.FetchOne(
		ctx.Krt,
		ctx.Collections.BackendTLSPolicies,
		krt.FilterIndex(ctx.Collections.BackendTLSPoliciesByTargetRef, utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{
				Name:      name,
				Namespace: namespace,
			},
			Kind: kind,
		}),
	))
}

func tlsConfigFromBackendTLSPolicy(ctx PolicyCtx, namespace string, policy *gwv1.BackendTLSPolicy) (*tls.Config, error) {
	tlsConfig := &tls.Config{
		MinVersion: tls.VersionTLS12,
		ServerName: string(policy.Spec.Validation.Hostname),
	}
	if len(policy.Spec.Validation.CACertificateRefs) == 0 {
		return tlsConfig, nil
	}

	certPool := x509.NewCertPool()
	for _, ref := range policy.Spec.Validation.CACertificateRefs {
		nn := types.NamespacedName{
			Name:      string(ref.Name),
			Namespace: namespace,
		}
		cfgmap := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterObjectName(nn)))
		if cfgmap == nil {
			return nil, fmt.Errorf("ConfigMap %s not found", nn)
		}
		if !jwks_url.AppendPoolWithCertsFromConfigMap(certPool, cfgmap) {
			return nil, fmt.Errorf("error extracting CA cert from ConfigMap %s", nn)
		}
	}
	tlsConfig.RootCAs = certPool
	return tlsConfig, nil
}
