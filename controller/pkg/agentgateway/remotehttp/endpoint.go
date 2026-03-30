package remotehttp

import "crypto/tls"

type FetchTarget struct {
	URL       string               `json:"url"`
	Transport TransportFingerprint `json:"transport,omitempty"`
}

type ResolvedTarget struct {
	Key       FetchKey
	Target    FetchTarget
	TLSConfig *tls.Config
}
