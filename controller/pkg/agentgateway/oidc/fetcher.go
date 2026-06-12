package oidc

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type discoveryDocument struct {
	Issuer                            string   `json:"issuer"`
	AuthorizationEndpoint             string   `json:"authorization_endpoint"`
	TokenEndpoint                     string   `json:"token_endpoint"`
	JwksURI                           string   `json:"jwks_uri"`
	TokenEndpointAuthMethodsSupported []string `json:"token_endpoint_auth_methods_supported"`
}

// Fetcher fetches and periodically refreshes OIDC discovery documents.
// Fetched providers are published as KRT-visible Results.
type Fetcher = remotecache.Fetcher[SharedOidcRequest, DiscoveredProvider]

// NewFetcher constructs a Fetcher backed by a fresh OidcDriver. The driver is
// returned so tests can swap its DefaultClient.
func NewFetcher(results *OidcResults) (*Fetcher, *OidcDriver) {
	driver := &OidcDriver{DefaultClient: remotehttp.NewDefaultFetchClient()}
	return remotecache.NewFetcher[SharedOidcRequest, DiscoveredProvider](results, driver.Fetch, logger), driver
}

type OidcDriver struct {
	DefaultClient *http.Client
}

func (d *OidcDriver) Fetch(ctx context.Context, source SharedOidcRequest) (DiscoveredProvider, error) {
	// PickClient honors per-source TLSConfig + ProxyTLSConfig populated by the
	// resolver from an attached AgentgatewayPolicy. The same client is used for
	// JWKS so backendRef TLS/proxy settings apply consistently across discovery
	// and key material fetches.
	client, err := remotehttp.PickClient(d.DefaultClient, source.Target, source.TLSConfig, source.ProxyTLSConfig)
	if err != nil {
		return DiscoveredProvider{}, err
	}

	logger.InfoContext(ctx, "fetching oidc discovery document", "url", source.Target.URL)

	doc, err := remotehttp.FetchJSON[discoveryDocument](ctx, client, source.Target, "OIDC discovery")
	if err != nil {
		return DiscoveredProvider{}, err
	}
	if err := validateDiscoveryDocument(doc, source.ExpectedIssuer); err != nil {
		return DiscoveredProvider{}, err
	}

	jwksURL, err := oidcJWKSFetchURL(source, doc.JwksURI)
	if err != nil {
		return DiscoveredProvider{}, err
	}

	logger.InfoContext(ctx, "fetching oidc jwks", "url", doc.JwksURI, "fetchURL", jwksURL)
	jwksBody, _, err := remotehttp.FetchJWKSBody(ctx, client, jwksURL, "OIDC JWKS")
	if err != nil {
		return DiscoveredProvider{}, fmt.Errorf("failed to fetch OIDC JWKS from %s: %w", doc.JwksURI, err)
	}

	return DiscoveredProvider{
		RequestKey:                        source.RequestKey,
		IssuerURL:                         doc.Issuer,
		AuthorizationEndpoint:             doc.AuthorizationEndpoint,
		TokenEndpoint:                     doc.TokenEndpoint,
		JwksURI:                           doc.JwksURI,
		JwksInline:                        string(jwksBody),
		TokenEndpointAuthMethodsSupported: doc.TokenEndpointAuthMethodsSupported,
		FetchedAt:                         time.Now(),
	}, nil
}

func oidcJWKSFetchURL(source SharedOidcRequest, jwksURI string) (string, error) {
	if !source.ViaBackendRef {
		return jwksURI, nil
	}

	targetURL, err := url.Parse(source.Target.URL)
	if err != nil {
		return "", fmt.Errorf("invalid resolved OIDC provider backend URL %q: %w", source.Target.URL, err)
	}
	if !targetURL.IsAbs() || targetURL.Host == "" {
		return "", fmt.Errorf("resolved OIDC provider backend URL %q must be absolute with a host", source.Target.URL)
	}
	jwksURL, err := url.Parse(jwksURI)
	if err != nil {
		return "", fmt.Errorf("invalid OIDC jwks_uri %q: %w", jwksURI, err)
	}

	rewritten := *targetURL
	rewritten.Path = jwksURL.Path
	rewritten.RawPath = jwksURL.RawPath
	rewritten.RawQuery = jwksURL.RawQuery
	rewritten.ForceQuery = jwksURL.ForceQuery
	rewritten.Fragment = ""
	return rewritten.String(), nil
}

func validateDiscoveryDocument(doc discoveryDocument, expectedIssuer string) error {
	if doc.Issuer != expectedIssuer {
		return fmt.Errorf("issuer mismatch: discovery document reports %q but expected %q", doc.Issuer, expectedIssuer)
	}
	if err := validateAbsoluteHTTPSURL(doc.AuthorizationEndpoint, "authorization_endpoint"); err != nil {
		return err
	}
	if err := validateAbsoluteHTTPSURL(doc.TokenEndpoint, "token_endpoint"); err != nil {
		return err
	}
	if err := validateAbsoluteHTTPSURL(doc.JwksURI, "jwks_uri"); err != nil {
		return err
	}
	return nil
}

func validateAbsoluteHTTPSURL(raw, field string) error {
	if raw == "" {
		return fmt.Errorf("discovery document missing %s", field)
	}
	u, err := url.Parse(raw)
	if err != nil || !u.IsAbs() || u.Scheme != "https" || u.Host == "" {
		return fmt.Errorf("discovery document %s must be an absolute HTTPS URL", field)
	}
	return nil
}
