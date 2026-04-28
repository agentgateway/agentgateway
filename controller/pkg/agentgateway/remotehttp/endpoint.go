package remotehttp

import "crypto/tls"

type FetchTarget struct {
	URL            string               `json:"url"`
	Transport      TransportFingerprint `json:"transport,omitempty"`
	ProxyURL       string               `json:"proxyURL,omitempty"`
	ProxyTransport TransportFingerprint `json:"proxyTransport,omitempty"`
}

type ResolvedTarget struct {
	Key            FetchKey
	Target         FetchTarget
	TLSConfig      *tls.Config
	ProxyTLSConfig *tls.Config
}
