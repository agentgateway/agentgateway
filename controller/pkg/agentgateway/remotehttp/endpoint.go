package remotehttp

import "crypto/tls"

type Request struct {
	URL       string               `json:"url"`
	Transport TransportFingerprint `json:"transport,omitempty"`
}

type ResolvedEndpoint struct {
	Key       FetchKey
	Request   Request
	TLSConfig *tls.Config
}
