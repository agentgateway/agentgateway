package oidc

import (
	"encoding/json"
	"errors"
	"sync"

	"github.com/go-jose/go-jose/v4"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// OidcCache stores discovered OIDC providers by request key.
type OidcCache struct {
	l         sync.Mutex
	providers map[remotehttp.FetchKey]DiscoveredProvider
}

func NewCache() *OidcCache {
	return &OidcCache{
		providers: make(map[remotehttp.FetchKey]DiscoveredProvider),
	}
}

func (c *OidcCache) LoadProvidersFromStores(stored []DiscoveredProvider) error {
	newCache := NewCache()
	errs := make([]error, 0)

	for _, provider := range stored {
		var jwks jose.JSONWebKeySet
		if err := json.Unmarshal([]byte(provider.JwksJSON), &jwks); err != nil {
			errs = append(errs, err)
			continue
		}
		newCache.providers[provider.RequestKey] = provider
	}

	c.l.Lock()
	c.providers = newCache.providers
	c.l.Unlock()
	return errors.Join(errs...)
}

func (c *OidcCache) GetProvider(requestKey remotehttp.FetchKey) (DiscoveredProvider, bool) {
	c.l.Lock()
	defer c.l.Unlock()

	provider, ok := c.providers[requestKey]
	return provider, ok
}

func (c *OidcCache) putProvider(provider DiscoveredProvider) {
	c.l.Lock()
	defer c.l.Unlock()
	c.providers[provider.RequestKey] = provider
}

func (c *OidcCache) deleteProvider(requestKey remotehttp.FetchKey) bool {
	c.l.Lock()
	defer c.l.Unlock()
	_, existed := c.providers[requestKey]
	delete(c.providers, requestKey)
	return existed
}

func (c *OidcCache) Keys() []remotehttp.FetchKey {
	c.l.Lock()
	defer c.l.Unlock()
	keys := make([]remotehttp.FetchKey, 0, len(c.providers))
	for k := range c.providers {
		keys = append(keys, k)
	}
	return keys
}
