package oidc

import (
	"maps"
	"sync"
)

type providerCache struct {
	l         sync.Mutex
	providers map[string]StoredProvider
}

func NewProviderCache() *providerCache {
	return &providerCache{
		providers: make(map[string]StoredProvider),
	}
}

func (c *providerCache) LoadProvidersFromStores(stored map[string]StoredProvider) {
	newCache := NewProviderCache()
	maps.Copy(newCache.providers, stored)

	c.l.Lock()
	c.providers = newCache.providers
	c.l.Unlock()
}

func (c *providerCache) GetProvider(key string) (StoredProvider, bool) {
	c.l.Lock()
	defer c.l.Unlock()

	provider, ok := c.providers[key]
	return provider, ok
}

func (c *providerCache) AddProvider(key string, provider StoredProvider) {
	c.l.Lock()
	c.providers[key] = provider
	c.l.Unlock()
}

func (c *providerCache) DeleteProvider(key string) {
	c.l.Lock()
	delete(c.providers, key)
	c.l.Unlock()
}
