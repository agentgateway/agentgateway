package remotecache

import (
	"sync"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// MapCache is a thread-safe in-memory cache of remote fetch results keyed by
// FetchKey. Implements Cache[R].
type MapCache[R Result] struct {
	mu      sync.Mutex
	entries map[remotehttp.FetchKey]R
}

// NewMapCache constructs an empty MapCache[R].
func NewMapCache[R Result]() *MapCache[R] {
	return &MapCache[R]{entries: make(map[remotehttp.FetchKey]R)}
}

func (c *MapCache[R]) Get(key remotehttp.FetchKey) (R, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	r, ok := c.entries[key]
	return r, ok
}

func (c *MapCache[R]) Put(result R) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries[result.RemoteRequestKey()] = result
}

func (c *MapCache[R]) Delete(key remotehttp.FetchKey) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	_, existed := c.entries[key]
	delete(c.entries, key)
	return existed
}

func (c *MapCache[R]) Keys() []remotehttp.FetchKey {
	c.mu.Lock()
	defer c.mu.Unlock()
	keys := make([]remotehttp.FetchKey, 0, len(c.entries))
	for k := range c.entries {
		keys = append(keys, k)
	}
	return keys
}
