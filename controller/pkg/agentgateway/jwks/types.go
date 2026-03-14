package jwks

import (
	"crypto/tls"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type RequestKey = remotehttp.FetchKey
type Request = remotehttp.Request
type Transport = remotehttp.TransportFingerprint

type Artifact struct {
	RequestKey RequestKey `json:"requestKey"`
	URL        string     `json:"url"`
	FetchedAt  time.Time  `json:"fetchedAt"`
	JwksJSON   string     `json:"jwks"`
}

type JwksSource struct {
	OwnerKey   OwnerKey
	RequestKey RequestKey
	Request    Request
	TLSConfig  *tls.Config
	TTL        time.Duration
	Deleted    bool
}

func (s JwksSource) ResourceName() string {
	return s.OwnerKey.String()
}

type storedArtifact struct {
	Version int `json:"version"`
	Artifact
}
