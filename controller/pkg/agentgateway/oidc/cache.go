package oidc

import (
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
)

// OidcCache stores discovered OIDC providers by request key.
type OidcCache = remotecache.MapCache[DiscoveredProvider]

// NewCache constructs an empty OidcCache.
func NewCache() *OidcCache {
	return remotecache.NewMapCache[DiscoveredProvider]()
}
