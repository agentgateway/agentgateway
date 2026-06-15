package remotehttp

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"time"

	"github.com/go-jose/go-jose/v4"
)

const clientTimeout = 10 * time.Second

// NewDefaultFetchClient returns an http.Client configured with the package's
// shared timeouts and TLS defaults. It is equivalent to newFetchClient with
// no per-target TLS or proxy overrides, but cannot fail and so removes the
// silent-error pitfall at constructor sites.
func NewDefaultFetchClient() *http.Client {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	return &http.Client{
		Timeout: clientTimeout,
		Transport: &http.Transport{
			DialContext:       dialer.DialContext,
			DisableKeepAlives: true,
		},
	}
}

func newFetchClient(tlsConfig *tls.Config, proxyURL string, proxyTLSConfig *tls.Config) (*http.Client, error) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	transport := &http.Transport{
		TLSClientConfig:   tlsConfig,
		DialContext:       dialer.DialContext,
		DisableKeepAlives: true,
	}
	if proxyURL != "" {
		parsed, err := url.Parse(proxyURL)
		if err != nil {
			return nil, fmt.Errorf("error parsing proxy URL %q: %w", proxyURL, err)
		}
		if proxyTLSConfig != nil {
			// Downgrade the proxy URL scheme to http so that Go's transport
			// does not attempt its own TLS handshake to the proxy. Our custom
			// DialContext handles TLS with the proxy-specific configuration.
			httpProxy := *parsed
			httpProxy.Scheme = "http"
			transport.Proxy = http.ProxyURL(&httpProxy)
			transport.DialContext = proxyTLSDialContext(dialer, proxyTLSConfig)
		} else {
			transport.Proxy = http.ProxyURL(parsed)
		}
	}
	return &http.Client{
		Timeout:   clientTimeout,
		Transport: transport,
	}, nil
}

// proxyTLSDialContext returns a DialContext function that wraps TCP connections
// in TLS using the given proxy TLS configuration. This is used when the tunnel
// proxy backend has a TLS policy, so the CONNECT request is sent over TLS.
func proxyTLSDialContext(dialer *net.Dialer, proxyTLSConfig *tls.Config) func(ctx context.Context, network, addr string) (net.Conn, error) {
	return func(ctx context.Context, network, addr string) (net.Conn, error) {
		conn, err := dialer.DialContext(ctx, network, addr)
		if err != nil {
			return nil, err
		}
		tlsConn := tls.Client(conn, proxyTLSConfig.Clone())
		if err := tlsConn.HandshakeContext(ctx); err != nil {
			conn.Close() //nolint:errcheck
			return nil, err
		}
		return tlsConn, nil
	}
}

// PickClient returns the appropriate http.Client for a fetch — either the
// supplied default client, or a per-source client constructed from TLS or
// proxy overrides.
func PickClient(defaultClient *http.Client, target FetchTarget, tlsConfig, proxyTLSConfig *tls.Config) (*http.Client, error) {
	if tlsConfig == nil && target.ProxyURL == "" {
		return defaultClient, nil
	}
	return newFetchClient(tlsConfig, target.ProxyURL, proxyTLSConfig)
}

func FetchJSON[T any](ctx context.Context, client *http.Client, target FetchTarget, description string) (T, error) {
	var out T
	body, err := fetchBody(ctx, client, target.URL, description)
	if err != nil {
		return out, err
	}
	if err := json.Unmarshal(body, &out); err != nil {
		return out, fmt.Errorf("could not decode %s: %w", description, err)
	}
	return out, nil
}

func fetchBody(ctx context.Context, client *http.Client, requestURL, description string) ([]byte, error) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, requestURL, nil)
	if err != nil {
		return nil, fmt.Errorf("could not build request to get %s: %w", description, err)
	}

	response, err := client.Do(request)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close() //nolint:errcheck

	if response.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("unexpected status code from %s at %s: %d", description, requestURL, response.StatusCode)
	}

	body, err := io.ReadAll(io.LimitReader(response.Body, 1<<20))
	if err != nil {
		return nil, fmt.Errorf("could not read %s response: %w", description, err)
	}
	return body, nil
}

// FetchJWKSBody fetches a JWKS document, validates that it is syntactically a
// JWKS and contains at least one key, and returns both the original bytes and
// parsed keyset. The original bytes are important for OIDC, which ships the
// IdP's JWKS material downstream verbatim; the parsed value is used by direct
// JWKS consumers.
func FetchJWKSBody(ctx context.Context, client *http.Client, requestURL, description string) ([]byte, jose.JSONWebKeySet, error) {
	body, err := fetchBody(ctx, client, requestURL, description)
	if err != nil {
		return nil, jose.JSONWebKeySet{}, err
	}
	keyset, err := validateJWKSBody(body, requestURL, description)
	if err != nil {
		return nil, jose.JSONWebKeySet{}, err
	}
	return body, keyset, nil
}

func validateJWKSBody(body []byte, requestURL, description string) (jose.JSONWebKeySet, error) {
	var keyset jose.JSONWebKeySet
	if err := json.Unmarshal(body, &keyset); err != nil {
		return jose.JSONWebKeySet{}, fmt.Errorf("%s response from %s is not a valid JWKS document: %w", description, requestURL, err)
	}
	if len(keyset.Keys) == 0 {
		return jose.JSONWebKeySet{}, fmt.Errorf("%s response from %s contains no keys", description, requestURL)
	}
	return keyset, nil
}
