package curl

import (
	"encoding/base64"
	"net/http"
	"strconv"
	"strings"
)

// Option represents an option for a curl request.
type Option func(config *requestConfig)

// VerboseOutput returns the Option to emit a verbose output for the  curl request
// https://curl.se/docs/manpage.html#-v
func VerboseOutput() Option {
	return func(config *requestConfig) {
		config.verbose = true
	}
}

// Silent returns the Option to enable silent mode for the curl request
// https://curl.se/docs/manpage.html#-s
func Silent() Option {
	return func(config *requestConfig) {
		config.silent = true
	}
}

// WithMethod returns the Option to set the method for the curl request
// https://curl.se/docs/manpage.html#-X
func WithMethod(method string) Option {
	return func(config *requestConfig) {
		config.method = method
	}
}

// WithPort returns the Option to set the port for the curl request
func WithPort(port int) Option {
	return func(config *requestConfig) {
		config.port = port
	}
}

// WithConnectionTimeout returns the Option to set connect and request timeout in seconds.
func WithConnectionTimeout(seconds int) Option {
	return func(config *requestConfig) {
		config.connectionTimeout = seconds
	}
}

// WithHost returns the Option to set the host for the curl request
func WithHost(host string) Option {
	return func(config *requestConfig) {
		config.host = host
	}
}

// WithHostPort returns the Option to set the host and port for the curl request
// The provided string is assumed to have the format [HOST]:[PORT]
func WithHostPort(hostPort string) Option {
	return func(config *requestConfig) {
		parts := strings.Split(hostPort, ":")
		host := "unset"
		port := 0
		if len(parts) == 2 {
			host = parts[0]
			port, _ = strconv.Atoi(parts[1])
		}

		WithHost(host)(config)
		WithPort(port)(config)
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

// WithRetries returns the Option to configure the retries for the curl request
// https://curl.se/docs/manpage.html#--retry
// https://curl.se/docs/manpage.html#--retry-delay
// https://curl.se/docs/manpage.html#--retry-max-time
func WithRetries(retry, retryDelay, retryMaxTime int) Option {
	return func(config *requestConfig) {
		config.retry = retry
		config.retryDelay = retryDelay
		config.retryMaxTime = retryMaxTime
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

// WithBasicAuth returns the Option to configure a basic auth header for the curl request
func WithBasicAuth(username string, password string) Option {
	auth := username + ":" + password
	basicAuth := base64.StdEncoding.EncodeToString([]byte(auth))
	return func(config *requestConfig) {
		WithHeader("Authorization", "Basic "+basicAuth)(config)
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
