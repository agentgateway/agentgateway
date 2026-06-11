package curl

import (
	"bytes"
	"fmt"
	"io"
	"net/http"
	"strings"
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
		host:    "127.0.0.1",
		port:    80,
		headers: make(map[string][]string),
		scheme:  "http",
	}

	for _, opt := range options {
		opt(config)
	}

	return config.executeNative()
}

func (c *requestConfig) executeNative() (*http.Response, error) {
	fullURL := c.buildURL()

	client := &http.Client{
		Timeout: c.timeout,
		Transport: &http.Transport{
			DisableKeepAlives: true,
		},
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
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
