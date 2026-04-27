package jwks

import (
	"crypto/tls"
	"reflect"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type Keyset struct {
	RequestKey remotehttp.FetchKey `json:"requestKey"`
	URL        string              `json:"url"`
	FetchedAt  time.Time           `json:"fetchedAt"`
	JwksJSON   string              `json:"jwks"`
}

// JwksSource is a per-owner JWKS request before KRT collapses equivalent
// sources onto a shared request key.
type JwksSource struct {
	OwnerKey   OwnerKey
	RequestKey remotehttp.FetchKey
	Target     remotehttp.FetchTarget
	// +noKrtEquals
	TLSConfig *tls.Config
	// +noKrtEquals
	ProxyTLSConfig *tls.Config
	TTL            time.Duration
}

func (s JwksSource) ResourceName() string {
	return s.OwnerKey.String()
}

func (s JwksSource) Equals(other JwksSource) bool {
	return s.OwnerKey == other.OwnerKey &&
		s.RequestKey == other.RequestKey &&
		reflect.DeepEqual(s.Target, other.Target) &&
		s.TTL == other.TTL
}

// SharedJwksRequest is the canonical JWKS request produced by KRT for a shared
// fetch key. It is the unit the runtime Fetcher and persistence layer watch.
type SharedJwksRequest struct {
	RequestKey remotehttp.FetchKey
	Target     remotehttp.FetchTarget
	// +noKrtEquals
	TLSConfig *tls.Config
	// +noKrtEquals
	ProxyTLSConfig *tls.Config
	TTL            time.Duration
}

func (r SharedJwksRequest) ResourceName() string {
	return string(r.RequestKey)
}

func (r SharedJwksRequest) Equals(other SharedJwksRequest) bool {
	return r.RequestKey == other.RequestKey &&
		reflect.DeepEqual(r.Target, other.Target) &&
		r.TTL == other.TTL
}

// JwksSource returns the canonical runtime request consumed by the Fetcher.
func (r SharedJwksRequest) JwksSource() JwksSource {
	return JwksSource{
		RequestKey:     r.RequestKey,
		Target:         r.Target,
		TLSConfig:      r.TLSConfig,
		ProxyTLSConfig: r.ProxyTLSConfig,
		TTL:            r.TTL,
	}
}
