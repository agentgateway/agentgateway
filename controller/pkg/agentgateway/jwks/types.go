package jwks

import (
	"crypto/tls"
	"encoding/json"
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type Keyset struct {
	RequestKey remotehttp.FetchKey `json:"requestKey"`
	URL        string              `json:"url"`
	FetchedAt  time.Time           `json:"fetchedAt"`
	JwksJSON   string              `json:"jwks"`
}

func (k Keyset) RemoteRequestKey() remotehttp.FetchKey { return k.RequestKey }
func (k Keyset) RemoteFetchedAt() time.Time            { return k.FetchedAt }

func (k Keyset) ResourceName() string { return string(k.RequestKey) }

// Equals: field-by-field avoids reflect.DeepEqual on every KRT diff;
// FetchedAt uses time.Equal to ignore monotonic clock state.
func (k Keyset) Equals(other Keyset) bool {
	return k.RequestKey == other.RequestKey &&
		k.URL == other.URL &&
		k.FetchedAt.Equal(other.FetchedAt) &&
		k.JwksJSON == other.JwksJSON
}

var (
	_ krt.ResourceNamer   = Keyset{}
	_ krt.Equaler[Keyset] = Keyset{}
)

// jwksRequestSpec is the shared identity of a JWKS request: the fields that
// decide what to fetch and how to reach it. JwksSource adds the per-owner key;
// SharedJwksRequest is the collapsed canonical form. Both embed this so a field
// added here automatically participates in equality, the krt snapshot JSON, and
// the runtime request for both — no lockstep duplication.
type jwksRequestSpec struct {
	RequestKey remotehttp.FetchKey
	Target     remotehttp.FetchTarget
	// +noKrtEquals
	TLSConfig *tls.Config
	// +noKrtEquals
	ProxyTLSConfig *tls.Config
	TTL            time.Duration
}

func (s jwksRequestSpec) RemoteRequestKey() remotehttp.FetchKey { return s.RequestKey }
func (s jwksRequestSpec) RemoteTTL() time.Duration              { return s.TTL }

// equals avoids reflect.DeepEqual on every KRT diff.
func (s jwksRequestSpec) equals(other jwksRequestSpec) bool {
	return s.RequestKey == other.RequestKey &&
		s.Target.Equals(other.Target) &&
		s.TTL == other.TTL
}

// jwksRequestJSON is the krt-snapshot view of a spec: the embedded *tls.Config
// objects are not serializable, so their presence is reported as booleans.
type jwksRequestJSON struct {
	RequestKey        remotehttp.FetchKey    `json:"requestKey"`
	Target            remotehttp.FetchTarget `json:"target"`
	HasTLSConfig      bool                   `json:"hasTLSConfig"`
	HasProxyTLSConfig bool                   `json:"hasProxyTLSConfig"`
	TTL               time.Duration          `json:"ttl"`
}

func (s jwksRequestSpec) snapshot() jwksRequestJSON {
	return jwksRequestJSON{
		RequestKey:        s.RequestKey,
		Target:            s.Target,
		HasTLSConfig:      s.TLSConfig != nil,
		HasProxyTLSConfig: s.ProxyTLSConfig != nil,
		TTL:               s.TTL,
	}
}

// JwksSource is a per-owner JWKS request before KRT collapses equivalent
// sources onto a shared request key.
type JwksSource struct {
	OwnerKey remotecache.OwnerID
	jwksRequestSpec
}

func (s JwksSource) ResourceName() string { return s.OwnerKey.String() }

func (s JwksSource) Equals(other JwksSource) bool {
	return s.OwnerKey == other.OwnerKey && s.jwksRequestSpec.equals(other.jwksRequestSpec)
}

func (s JwksSource) MarshalJSON() ([]byte, error) {
	return json.Marshal(struct {
		OwnerKey remotecache.OwnerID `json:"ownerKey"`
		jwksRequestJSON
	}{OwnerKey: s.OwnerKey, jwksRequestJSON: s.jwksRequestSpec.snapshot()})
}

// SharedJwksRequest is the canonical JWKS request produced by KRT for a shared
// fetch key. It is the unit the runtime Fetcher and persistence layer watch.
type SharedJwksRequest struct {
	jwksRequestSpec
}

func (r SharedJwksRequest) ResourceName() string { return string(r.RequestKey) }

func (r SharedJwksRequest) Equals(other SharedJwksRequest) bool {
	return r.jwksRequestSpec.equals(other.jwksRequestSpec)
}

func (r SharedJwksRequest) MarshalJSON() ([]byte, error) {
	return json.Marshal(r.jwksRequestSpec.snapshot())
}
