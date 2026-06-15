package remotehttp

import (
	"crypto/tls"

	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

type FetchTarget struct {
	URL            string               `json:"url"`
	Transport      TransportFingerprint `json:"transport"`
	ProxyURL       string               `json:"proxyURL,omitempty"`
	ProxyTransport TransportFingerprint `json:"proxyTransport"`
}

// Equals avoids reflect.DeepEqual on the KRT diff hot path.
func (t FetchTarget) Equals(other FetchTarget) bool {
	return t.URL == other.URL &&
		t.ProxyURL == other.ProxyURL &&
		t.Transport.Equals(other.Transport) &&
		t.ProxyTransport.Equals(other.ProxyTransport)
}

type ResolvedTarget struct {
	Target         FetchTarget
	TLSConfig      *tls.Config
	ProxyTLSConfig *tls.Config
}

type TransportFingerprint struct {
	// Zero value means strict/default verification.
	Verification agentgateway.InsecureTLSMode `json:"verification,omitempty"`
	ServerName   string                       `json:"serverName,omitempty"`
	CABundleHash string                       `json:"caBundleHash,omitempty"`
	NextProtos   []string                     `json:"nextProtos,omitempty"`
}

// Equals avoids reflect.DeepEqual on the KRT diff hot path.
func (t TransportFingerprint) Equals(other TransportFingerprint) bool {
	return t.Verification == other.Verification &&
		t.ServerName == other.ServerName &&
		t.CABundleHash == other.CABundleHash &&
		slices.Equal(t.NextProtos, other.NextProtos)
}
