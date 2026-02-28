package curl

import (
	"bytes"
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// ExecuteRequest accepts a set of Option and executes a native Go HTTP request
// If multiple Option modify the same parameter, the last defined one will win
//
// Example:
//
//	resp, err := ExecuteRequest(WithMethod("GET"), WithMethod("POST"))
//	will executeNative a POST request
//
// A notable exception is the WithHeader option, which accumulates headers
func ExecuteRequest(options ...Option) (*http.Response, error) {
	config := &requestConfig{
		verbose:           false,
		ignoreServerCert:  false,
		connectionTimeout: 0,
		headersOnly:       false,
		method:            "",
		host:              "127.0.0.1",
		port:              80,
		headers:           make(map[string][]string),
		scheme:            "http",
		sni:               "",
		caFile:            "",
		path:              "",
		retry:             0,
		retryDelay:        -1,
		retryMaxTime:      0,
		ipv4Only:          false,
		ipv6Only:          false,
		cookie:            "",
		queryParameters:   make(map[string]string),
	}

	for _, opt := range options {
		opt(config)
	}

	return config.executeNative()
}

func (c *requestConfig) executeNative() (*http.Response, error) {
	// Build URL
	fullURL := c.buildURL()

	// Create HTTP client with custom transport
	client := c.buildHTTPClient()

	method := c.method

	// Prepare request body
	var bodyReader io.Reader
	if c.body != "" {
		bodyReader = bytes.NewBufferString(c.body)
		if method == "" {
			method = "POST"
		}
	}

	// Create context with timeout
	ctx := context.Background()
	var cancel context.CancelFunc
	if c.connectionTimeout > 0 {
		ctx, cancel = context.WithTimeout(ctx, time.Duration(c.connectionTimeout)*time.Second)
	}
	if method == "" {
		method = "GET"
	}

	// Create request
	req, err := http.NewRequestWithContext(ctx, method, fullURL, bodyReader)
	if err != nil {
		if cancel != nil {
			cancel()
		}
		return nil, fmt.Errorf("failed to create request: %w", err)
	}

	// Add headers
	for key, values := range c.headers {
		for _, value := range values {
			// Host header must be set on req.Host, not in req.Header
			if strings.EqualFold(key, "Host") {
				req.Host = value
			} else {
				req.Header.Add(key, value)
			}
		}
	}

	// Add cookies
	if c.cookie != "" {
		req.Header.Add("Cookie", c.cookie)
	}

	// Handle HEAD-only requests
	if c.headersOnly {
		req.Method = "HEAD"
	}

	// Execute request
	if c.verbose {
		fmt.Printf("> %s %s\n", req.Method, req.URL.String())
		fmt.Printf("> Host: %s\n", req.Host)
		for k, v := range req.Header {
			fmt.Printf("> %s: %s\n", k, strings.Join(v, ", "))
		}
	}

	resp, err := client.Do(req)
	if err != nil {
		if cancel != nil {
			cancel()
		}
		if c.verbose {
			fmt.Printf("Request failed: %v\n", err)
		}
		return nil, err
	}

	// Keep timeout context alive while caller reads the body, then release it on close.
	if cancel != nil {
		resp.Body = &cancelOnCloseReadCloser{
			ReadCloser: resp.Body,
			cancel:     cancel,
		}
	}

	if c.verbose {
		fmt.Printf("< HTTP %s\n", resp.Status)
		for k, v := range resp.Header {
			fmt.Printf("< %s: %s\n", k, strings.Join(v, ", "))
		}
	}

	return resp, nil
}

type cancelOnCloseReadCloser struct {
	io.ReadCloser
	cancel context.CancelFunc
}

func (r *cancelOnCloseReadCloser) Close() error {
	if r.cancel != nil {
		r.cancel()
	}
	return r.ReadCloser.Close()
}

func (c *requestConfig) buildURL() string {
	path := c.path
	if path != "" && !strings.HasPrefix(path, "/") {
		path = "/" + path
	}

	baseURL := fmt.Sprintf("%s://%s:%d%s", c.scheme, c.host, c.port, path)

	if len(c.queryParameters) > 0 {
		values := url.Values{}
		for k, v := range c.queryParameters {
			values.Add(k, v)
		}
		return fmt.Sprintf("%s?%s", baseURL, values.Encode())
	}

	return baseURL
}

func (c *requestConfig) buildHTTPClient() *http.Client {
	transport := &http.Transport{
		DialContext: c.buildDialer(),
	}

	// Configure TLS
	if c.scheme == "https" || c.ignoreServerCert || c.sni != "" {
		tlsConfig := &tls.Config{
			InsecureSkipVerify: c.ignoreServerCert, // nolint: gosec // this is for tests
		}

		if c.sni != "" {
			tlsConfig.ServerName = c.sni
		}

		// Configure TLS version
		if c.tlsVersion != "" {
			tlsConfig.MinVersion = parseTLSVersion(c.tlsVersion)
		}
		if c.tlsMaxVersion != "" {
			tlsConfig.MaxVersion = parseTLSVersion(c.tlsMaxVersion)
		}

		// Configure cipher suites (simplified)
		if c.ciphers != "" {
			// Note: Go's TLS implementation uses predefined cipher suites
			// This would require parsing the cipher string and mapping to Go's constants
			// For simplicity, this is left as a placeholder
		}

		// Configure curves (simplified)
		if c.curves != "" {
			// Similar to ciphers, this would require parsing and mapping
		}

		transport.TLSClientConfig = tlsConfig
	}

	// Configure HTTP version
	if c.http2 {
		// HTTP/2 is enabled by default in Go's transport
		transport.ForceAttemptHTTP2 = true
	} else if c.http11 {
		// Disable HTTP/2 to force HTTP/1.1
		transport.ForceAttemptHTTP2 = false
		transport.TLSNextProto = make(map[string]func(string, *tls.Conn) http.RoundTripper)
	}

	client := &http.Client{
		Transport: transport,
	}

	// Set timeout (client-level timeout)
	if c.connectionTimeout > 0 {
		client.Timeout = time.Duration(c.connectionTimeout) * time.Second
	}

	client.CheckRedirect = func(req *http.Request, via []*http.Request) error {
		// Disable redirects
		return http.ErrUseLastResponse
	}
	return client
}

func (c *requestConfig) buildDialer() func(context.Context, string, string) (net.Conn, error) {
	dialer := &net.Dialer{
		Timeout: 30 * time.Second,
	}

	if c.connectionTimeout > 0 {
		dialer.Timeout = time.Duration(c.connectionTimeout) * time.Second
	}

	// Handle IPv4/IPv6 restrictions
	if c.ipv4Only {
		return func(ctx context.Context, network, addr string) (net.Conn, error) {
			return dialer.DialContext(ctx, "tcp4", addr)
		}
	}
	if c.ipv6Only {
		return func(ctx context.Context, network, addr string) (net.Conn, error) {
			return dialer.DialContext(ctx, "tcp6", addr)
		}
	}

	// Handle SNI with custom host resolution
	// TODO
	if c.sni != "" {
		panic("sni is not implemented")
	}

	return dialer.DialContext
}

func parseTLSVersion(version string) uint16 {
	switch version {
	case "1.0":
		return tls.VersionTLS10
	case "1.1":
		return tls.VersionTLS11
	case "1.2":
		return tls.VersionTLS12
	case "1.3":
		return tls.VersionTLS13
	default:
		return tls.VersionTLS12 // default
	}
}
