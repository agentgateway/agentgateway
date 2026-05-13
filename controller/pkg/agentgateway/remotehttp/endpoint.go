package remotehttp

import "crypto/tls"

type FetchTarget struct {
	URL            string               `json:"url"`
	Transport      TransportFingerprint `json:"transport"`
	ProxyURL       string               `json:"proxyURL,omitempty"`
	ProxyTransport TransportFingerprint `json:"proxyTransport"`
}

type ResolvedTarget struct {
	Key            FetchKey
	Target         FetchTarget
	TLSConfig      *tls.Config
	ProxyTLSConfig *tls.Config
}
