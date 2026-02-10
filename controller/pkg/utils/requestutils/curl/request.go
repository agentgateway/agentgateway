package curl

import (
	"fmt"
	"net/url"
)

// BuildArgs accepts a set of curl.Option and generates the list of arguments
// that can be used to execute a curl request
// If multiple Option modify the same argument, the last defined one will win:
//
//	Example:
//		BuildArgs(WithMethod("GET"), WithMethod("POST"))
//		will return a curl with using a post method
//
// A notable exception to this is the WithHeader option, which will always modify
// the map of headers used in the curl request.
func BuildArgs(options ...Option) []string {
	config := &requestConfig{
		verbose:          false,
		ignoreServerCert: false,
		silent:           false,
		// Do not set a default timeout here; let the caller (e.g., kubectl.Cli.CurlFromPod)
		// control defaults to avoid duplicate flags. Only add timeouts when explicitly requested.
		connectionTimeout: 0,
		headersOnly:       false,
		method:            "",
		host:              "127.0.0.1",
		port:              8080,
		headers:           make(map[string][]string),
		scheme:            "http", // https://github.com/golang/go/issues/40587
		sni:               "",
		caFile:            "",
		path:              "",
		retry:             0, // do not retry
		retryDelay:        -1,
		retryMaxTime:      0,
		ipv4Only:          false,
		ipv6Only:          false,
		cookie:            "",
		cookieJar:         "",

		additionalArgs: []string{},
	}

	for _, opt := range options {
		opt(config)
	}

	return config.generateArgs()
}

// requestConfig contains the set of options that can be used to configure a curl request
type requestConfig struct {
	verbose           bool
	ignoreServerCert  bool
	silent            bool
	connectionTimeout int // seconds
	headersOnly       bool
	method            string
	host              string
	port              int
	headers           map[string][]string
	body              string
	sni               string
	caFile            string
	path              string
	queryParameters   map[string]string

	cookie    string
	cookieJar string

	scheme string

	proxyProto bool

	retry                  int
	retryDelay             int
	retryMaxTime           int
	retryConnectionRefused bool

	ipv4Only bool
	ipv6Only bool

	ignoreBody bool
	// HTTP protocol options
	http11 bool
	http2  bool

	// TLS-specific options
	ciphers       string
	curves        string
	tlsVersion    string
	tlsMaxVersion string

	// Client certificate options
	clientCert string
	clientKey  string

	additionalArgs []string
}

func (c *requestConfig) generateArgs() []string {
	var args []string

	if c.verbose {
		args = append(args, "-v")
	}
	if c.ignoreServerCert {
		args = append(args, "-k")
	}
	if c.silent {
		args = append(args, "-s")
	}
	if c.connectionTimeout > 0 {
		seconds := fmt.Sprintf("%v", c.connectionTimeout)
		args = append(args, "--connect-timeout", seconds, "--max-time", seconds)
	}
	if c.headersOnly {
		args = append(args, "-I")
	}
	if c.method != "" {
		args = append(args, "--request", c.method)
	}

	for h, values := range c.headers {
		for _, v := range values {
			args = append(args, "-H", fmt.Sprintf("%v: %v", h, v))
		}
	}
	if c.caFile != "" {
		args = append(args, "--cacert", c.caFile)
	}
	if c.body != "" {
		args = append(args, "--data-binary", c.body)
	}
	if c.retry != 0 {
		args = append(args, "--retry", fmt.Sprintf("%d", c.retry))
	}
	if c.retryDelay != -1 {
		args = append(args, "--retry-delay", fmt.Sprintf("%d", c.retryDelay))
	}
	if c.retryMaxTime != 0 {
		args = append(args, "--retry-max-time", fmt.Sprintf("%d", c.retryMaxTime))
	}
	if c.retryConnectionRefused {
		args = append(args, "--retry-connrefused")
	}

	if c.proxyProto {
		args = append(args, "--haproxy-protocol")
	}
	// HTTP protocol options
	if c.http11 {
		args = append(args, "--http1.1")
	}
	if c.http2 {
		args = append(args, "--http2")
	}

	// TLS-specific options
	if c.ciphers != "" {
		args = append(args, "--ciphers", c.ciphers)
	}
	if c.curves != "" {
		args = append(args, "--curves", c.curves)
	}
	if c.tlsVersion != "" {
		args = append(args, fmt.Sprintf("--tlsv%s", c.tlsVersion))
	}
	if c.tlsMaxVersion != "" {
		args = append(args, "--tls-max", c.tlsMaxVersion)
	}

	// Client certificate options
	if c.clientCert != "" {
		args = append(args, "--cert", c.clientCert)
	}
	if c.clientKey != "" {
		args = append(args, "--key", c.clientKey)
	}

	if len(c.additionalArgs) > 0 {
		args = append(args, c.additionalArgs...)
	}

	// Todo: rely on url.Url to construct the address
	var fullAddress string

	if c.sni != "" {
		sniResolution := fmt.Sprintf("%s:%d:%s:%d", c.sni, c.port, c.host, c.port)
		fullAddress = fmt.Sprintf("%s://%s:%d", c.scheme, c.sni, c.port)
		args = append(args, "--connect-to", sniResolution)
	} else {
		fullAddress = fmt.Sprintf("%v://%s:%v/%s", c.scheme, c.host, c.port, c.path)
		if len(c.queryParameters) > 0 {
			values := url.Values{}
			for k, v := range c.queryParameters {
				values.Add(k, v)
			}
			fullAddress = fmt.Sprintf("%s?%s", fullAddress, values.Encode())
		}
	}

	if c.cookie != "" {
		args = append(args, "--cookie", c.cookie)
	}

	if c.cookieJar != "" {
		args = append(args, "--cookie-jar", c.cookieJar)
	}

	if c.ignoreBody {
		args = append(args, "--output", "/dev/null")
	}

	args = append(args, fullAddress)
	return args
}
