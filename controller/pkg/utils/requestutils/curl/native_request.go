package curl

import (
	"bytes"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"
)

// Shared transport used for common requests so we reuse connections and TLS sessions.
// Only cloned/overridden when a custom TLS config is provided.
var sharedTransport http.RoundTripper

func init() {
	// Attempt to use default transport as basis and tune keep-alive settings.
	if t, ok := http.DefaultTransport.(*http.Transport); ok {
		st := t.Clone()
		// Tune sensible defaults for connection reuse. These can be adjusted later if needed.
		st.MaxIdleConns = 100
		st.MaxIdleConnsPerHost = 100
		st.IdleConnTimeout = 90 * time.Second
		// other fields (TLSHandshakeTimeout, ExpectContinueTimeout) remain as DefaultTransport's values
		sharedTransport = st
		return
	}

	// Preserve proxy/dial behavior of the default transport instead of falling back to a zero-value Transport.
	// http.DefaultTransport already implements http.RoundTripper, so use it as the safe fallback.
	sharedTransport = http.DefaultTransport
}

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
		host:    "127.0.0.1",
		port:    80,
		headers: make(map[string][]string),
		scheme:  "http",
		timeout: 0, // zero means no timeout (default behaviour)
	}

	for _, opt := range options {
		opt(config)
	}

	return config.executeNative()
}

func (c *requestConfig) executeNative() (*http.Response, error) {
	fullURL := c.buildURL()

	// Start with a client that uses the shared transport (connection reuse).
	client := &http.Client{
		Timeout:   c.timeout,
		Transport: sharedTransport,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}

	// If caller provided a TLS config, clone default transport and set TLSClientConfig.
	// This preserves previous behavior (per-request TLS transport) while keeping the
	// common path efficient.
	if c.tlsConfig != nil {
		if t, ok := http.DefaultTransport.(*http.Transport); ok {
			transport := t.Clone()
			transport.TLSClientConfig = c.tlsConfig
			client.Transport = transport
		} else {
			// Fall back to a fresh transport
			client.Transport = &http.Transport{
				TLSClientConfig: c.tlsConfig,
			}
		}
	}

	method := c.method

	var bodyReader io.Reader
	if c.body != "" {
		bodyReader = bytes.NewBufferString(c.body)
		if method == "" {
			method = "POST"
		}
	}

	if method == "" {
		method = "GET"
	}

	req, err := http.NewRequest(method, fullURL, bodyReader)
	if err != nil {
		return nil, fmt.Errorf("failed to create request: %w", err)
	}

	for key, values := range c.headers {
		for _, value := range values {
			if strings.EqualFold(key, "Host") {
				req.Host = value
			} else {
				req.Header.Add(key, value)
			}
		}
	}

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}

	return resp, nil
}

func (c *requestConfig) buildURL() string {
	path := c.path
	if path != "" && !strings.HasPrefix(path, "/") {
		path = "/" + path
	}

	baseURL := fmt.Sprintf("%s://%s:%d%s", c.scheme, c.host, c.port, path)
	return baseURL
}