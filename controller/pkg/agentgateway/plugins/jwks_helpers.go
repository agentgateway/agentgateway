package plugins

import (
	"crypto/tls"
	"fmt"
	"net/url"
	"strings"
	"time"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/oidc"
)

type oidcResolver interface {
	Resolve(ctx PolicyCtx, policyName, policyNamespace, issuer string, providerBackendRef *gwv1.BackendObjectReference) (*resolvedOIDCProvider, error)
}

type defaultOIDCResolver struct{}

// resolvedOIDCProvider is the normalized provider config consumed by policy translation.
// It is derived from discovery metadata plus a resolved inline JWKS payload.
type resolvedOIDCProvider struct {
	AuthorizationEndpoint             string
	TokenEndpoint                     string
	EndSessionEndpoint                string
	TokenEndpointAuthMethodsSupported []string
	JwksInline                        string
}

const oidcJWKSCacheTTL = 5 * time.Minute

// resolveRemoteJWKSInline resolves a remote JWKS URI to an inline JWKS string by
// looking up the JWKS store ConfigMap and extracting the serialized JWKS JSON.
func resolveRemoteJWKSInline(ctx PolicyCtx, jwksURI string) (string, error) {
	if _, err := url.Parse(jwksURI); err != nil {
		return "", fmt.Errorf("invalid jwks url %w", err)
	}
	jwksStoreName := jwks.JwksConfigMapNamespacedName(jwks.DefaultJwksStorePrefix, ctx.Collections.SystemNamespace, jwksURI)
	jwksCM := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterObjectName(jwksStoreName)))
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
	provider, err := resolveStoredOIDCProvider(ctx, policyNamespace, issuer, providerBackendRef)
	if err != nil {
		return nil, err
	}
	if provider.AuthorizationEndpoint == "" || provider.TokenEndpoint == "" || provider.JwksURI == "" {
		return nil, fmt.Errorf("oidc discovery document missing required endpoints for issuer %q", issuer)
	}
	jwksLookupURI := provider.JwksURI
	if providerBackendRef != nil {
		jwksLookupURI, _, _, err = buildProviderFetchTarget(
			ctx,
			policyName,
			policyNamespace,
			providerBackendRef,
			provider.JwksURI,
		)
		if err != nil {
			return nil, fmt.Errorf("failed resolving oauth2 provider jwks lookup url: %w", err)
		}
	}
	jwksInline, err := resolveRemoteJWKSInline(ctx, jwksLookupURI)
	if err != nil {
		return nil, fmt.Errorf("failed resolving oidc jwks from controller store: %w", err)
	}

	return &resolvedOIDCProvider{
		AuthorizationEndpoint:             provider.AuthorizationEndpoint,
		TokenEndpoint:                     provider.TokenEndpoint,
		EndSessionEndpoint:                provider.EndSessionEndpoint,
		TokenEndpointAuthMethodsSupported: provider.TokenEndpointAuthMethodsSupported,
		JwksInline:                        string(jwksInline),
	}, nil
}

func BuildOIDCProviderSource(
	ctx PolicyCtx,
	policyName, policyNamespace, issuer string,
	providerBackendRef *gwv1.BackendObjectReference,
) (*oidc.ProviderSource, error) {
	discoveryURL, err := oidc.BuildDiscoveryURL(issuer)
	if err != nil {
		return nil, err
	}
	requestURL, hostOverride, tlsConfig, err := buildProviderFetchTarget(
		ctx,
		policyName,
		policyNamespace,
		providerBackendRef,
		discoveryURL,
	)
	if err != nil {
		return nil, err
	}
	resourceKey, err := oidc.CanonicalSourceKey(issuer, policyNamespace, providerBackendRef)
	if err != nil {
		return nil, err
	}
	return &oidc.ProviderSource{
		ResourceKey:  resourceKey,
		Issuer:       issuer,
		RequestURL:   requestURL,
		HostOverride: hostOverride,
		TlsConfig:    tlsConfig,
		Ttl:          oidc.DefaultProviderStoreTTL,
	}, nil
}

func resolveStoredOIDCProvider(
	ctx PolicyCtx,
	policyNamespace string,
	issuer string,
	providerBackendRef *gwv1.BackendObjectReference,
) (*oidc.StoredProvider, error) {
	resourceKey, err := oidc.CanonicalSourceKey(issuer, policyNamespace, providerBackendRef)
	if err != nil {
		return nil, err
	}
	providerStoreName := oidc.ProviderConfigMapNamespacedName(resourceKey)
	if providerStoreName == nil {
		return nil, fmt.Errorf("oidc provider store hasn't been initialized")
	}
	providerCM := ptr.Flatten(krt.FetchOne(
		ctx.Krt,
		ctx.Collections.ConfigMaps,
		krt.FilterObjectName(*providerStoreName),
	))
	if providerCM == nil {
		return nil, fmt.Errorf("oidc provider ConfigMap %v isn't available", providerStoreName)
	}
	provider, err := oidc.ProviderFromConfigMap(providerCM)
	if err != nil {
		return nil, fmt.Errorf("error deserializing oidc provider ConfigMap %w", err)
	}
	if provider.ResourceKey != resourceKey {
		return nil, fmt.Errorf("oidc provider ConfigMap %v contains unexpected resource key", providerStoreName)
	}
	return &provider, nil
}

func ResolveOIDCJWKSSource(
	ctx PolicyCtx,
	policyName, policyNamespace, issuer string,
	providerBackendRef *gwv1.BackendObjectReference,
) (*jwks.JwksSource, error) {
	provider, err := resolveStoredOIDCProvider(ctx, policyNamespace, issuer, providerBackendRef)
	if err != nil {
		return nil, err
	}
	if provider.JwksURI == "" {
		return nil, fmt.Errorf("oidc discovery document missing required jwks_uri for issuer %q", issuer)
	}
	jwksURL, hostOverride, tlsConfig, err := buildProviderFetchTarget(ctx, policyName, policyNamespace, providerBackendRef, provider.JwksURI)
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
	lookup := ctx.Collections.BackendTransportLookup
	if lookup == nil {
		return "", nil, fmt.Errorf("backend transport lookup is not initialized")
	}
	transport, err := lookup.Resolve(ctx.Krt, policyName, policyNamespace, backendRef, targetURL.Port())
	if err != nil {
		return "", nil, err
	}

	scheme := targetURL.Scheme
	tlsConfig := transport.TLSConfig
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
		Host:     transport.ConnectHost,
		Path:     targetURL.EscapedPath(),
		RawPath:  targetURL.RawPath,
		RawQuery: targetURL.RawQuery,
	}
	return requestURL.String(), tlsConfig, nil
}
