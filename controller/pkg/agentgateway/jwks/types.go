package jwks

import (
	"crypto/tls"
	"encoding/json"
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

func (s JwksSource) MarshalJSON() ([]byte, error) {
	type jsonJwksSource struct {
		OwnerKey          OwnerKey               `json:"ownerKey"`
		RequestKey        remotehttp.FetchKey    `json:"requestKey"`
		Target            remotehttp.FetchTarget `json:"target"`
		HasTLSConfig      bool                   `json:"hasTLSConfig"`
		HasProxyTLSConfig bool                   `json:"hasProxyTLSConfig"`
		TTL               time.Duration          `json:"ttl"`
	}

	return json.Marshal(jsonJwksSource{
		OwnerKey:          s.OwnerKey,
		RequestKey:        s.RequestKey,
		Target:            s.Target,
		HasTLSConfig:      s.TLSConfig != nil,      // the underlying tls.Config is not serializable, so we will just indicate its presence
		HasProxyTLSConfig: s.ProxyTLSConfig != nil, // the underlying tls.Config is not serializable, so we will just indicate its presence
		TTL:               s.TTL,
	})
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

func (r SharedJwksRequest) MarshalJSON() ([]byte, error) {
	type jsonSharedJwksRequest struct {
		RequestKey        remotehttp.FetchKey    `json:"requestKey"`
		Target            remotehttp.FetchTarget `json:"target"`
		HasTLSConfig      bool                   `json:"hasTLSConfig"`
		HasProxyTLSConfig bool                   `json:"hasProxyTLSConfig"`
		TTL               time.Duration          `json:"ttl"`
	}

	return json.Marshal(jsonSharedJwksRequest{
		RequestKey:        r.RequestKey,
		Target:            r.Target,
		HasTLSConfig:      r.TLSConfig != nil,      // the underlying tls.Config is not serializable, so we will just indicate its presence
		HasProxyTLSConfig: r.ProxyTLSConfig != nil, // the underlying tls.Config is not serializable, so we will just indicate its presence
		TTL:               r.TTL,
	})
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
