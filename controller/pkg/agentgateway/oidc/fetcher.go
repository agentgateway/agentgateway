package oidc

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"time"

	"sigs.k8s.io/controller-runtime/pkg/log"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

var fetcherLogger = logging.New("oidc_fetcher")

type discoveryDocument struct {
	Issuer                            string   `json:"issuer"`
	AuthorizationEndpoint             string   `json:"authorization_endpoint"`
	TokenEndpoint                     string   `json:"token_endpoint"`
	JwksURI                           string   `json:"jwks_uri"`
	TokenEndpointAuthMethodsSupported []string `json:"token_endpoint_auth_methods_supported"`
}

// Fetcher fetches and periodically refreshes OIDC discovery documents.
type Fetcher = remotecache.Fetcher[SharedOidcRequest, DiscoveredProvider]

func NewFetcher(cache *OidcCache) *Fetcher {
	driver := &OidcDriver{DefaultClient: remotehttp.NewDefaultFetchClient()}
	return remotecache.NewFetcher[SharedOidcRequest, DiscoveredProvider](cache, driver, fetcherLogger)
}

type OidcDriver struct {
	DefaultClient *http.Client
}

func (d *OidcDriver) Fetch(ctx context.Context, source SharedOidcRequest) (DiscoveredProvider, error) {
	log.FromContext(ctx).Info("fetching oidc discovery document", "url", source.Target.URL)

	doc, err := remotehttp.FetchJSON[discoveryDocument](ctx, d.DefaultClient, source.Target, "OIDC discovery")
	if err != nil {
		return DiscoveredProvider{}, err
	}
	if err := validateDiscoveryDocument(doc, source.ExpectedIssuer); err != nil {
		return DiscoveredProvider{}, err
	}

	log.FromContext(ctx).Info("fetching oidc jwks", "url", doc.JwksURI)
	jwksBody, err := remotehttp.FetchBody(ctx, d.DefaultClient, doc.JwksURI, "OIDC JWKS")
	if err != nil {
		return DiscoveredProvider{}, fmt.Errorf("failed to fetch OIDC JWKS from %s: %w", doc.JwksURI, err)
	}
	if !json.Valid(jwksBody) {
		return DiscoveredProvider{}, fmt.Errorf("OIDC JWKS response from %s is not valid JSON", doc.JwksURI)
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
	if err := ValidateJwksURI(doc.JwksURI); err != nil {
		return err
	}
	return nil
}

func ValidateJwksURI(raw string) error {
	return validateAbsoluteHTTPSURL(raw, "jwks_uri")
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
