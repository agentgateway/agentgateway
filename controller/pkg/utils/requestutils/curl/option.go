package curl

import (
	"net/http"
	"strings"
	"time"
)

// Option represents an option for a curl request.
type Option func(config *requestConfig)

// WithMethod returns the Option to set the method for the curl request
// https://curl.se/docs/manpage.html#-X
func WithMethod(method string) Option {
	return func(config *requestConfig) {
		config.method = method
	}
}

// WithTimeout bounds the whole request via a context deadline.
func WithTimeout(d time.Duration) Option {
	return func(config *requestConfig) {
		config.timeout = d
	}
}

// WithPort returns the Option to set the port for the curl request
func WithPort(port int) Option {
	return func(config *requestConfig) {
		config.port = port
	}
}

// WithHost returns the Option to set the host for the curl request
func WithHost(host string) Option {
	return func(config *requestConfig) {
		config.host = host
	}
}

// WithPath returns the Option to configure the path of the curl request
// The provided path is expected to not contain a leading `/`,
// so if it is provided, it will be trimmed
func WithPath(path string) Option {
	return func(config *requestConfig) {
		config.path = strings.TrimPrefix(path, "/")
	}
}

// WithPostBody returns the Option to configure a curl request to execute a post request with the provided json body
func WithPostBody(body string) Option {
	return func(config *requestConfig) {
		WithMethod(http.MethodPost)(config)
		WithBody(body)(config)
		WithContentType("application/json")(config)
	}
}

// WithBody returns the Option to configure the body for a curl request
// https://curl.se/docs/manpage.html#-d
func WithBody(body string) Option {
	return func(config *requestConfig) {
		config.body = body
	}
}

// WithContentType returns the Option to configure the Content-Type header for the curl request
func WithContentType(contentType string) Option {
	return func(config *requestConfig) {
		WithHeader("Content-Type", contentType)(config)
	}
}

// WithHostHeader returns the Option to configure the Host header for the curl request
func WithHostHeader(host string) Option {
	return func(config *requestConfig) {
		WithHeader("Host", host)(config)
	}
}

// WithHeader returns the Option to configure a header for the curl request
// https://curl.se/docs/manpage.html#-H
func WithHeader(key, value string) Option {
	return func(config *requestConfig) {
		config.headers[key] = []string{value}
	}
}

// WithHeaders returns the Option to configure a list of headers for the curl request
func WithHeaders(headers map[string]string) Option {
	return func(config *requestConfig) {
		if config.headers == nil {
			config.headers = make(map[string][]string)
		}
		for h, v := range headers {
			config.headers[h] = []string{v}
		}
	}
}

// WithScheme returns the Option to configure the scheme for the curl request
func WithScheme(scheme string) Option {
	return func(config *requestConfig) {
		config.scheme = scheme
	}
}
