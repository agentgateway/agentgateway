package jwks

import (
	"encoding/json"
	"errors"
	"sync"
	"time"

	"github.com/go-jose/go-jose/v4"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// JwksCache stores fetched JWKS keysets by request key.
type JwksCache struct {
	l       sync.Mutex
	keysets map[remotehttp.FetchKey]Keyset
}

func NewCache() *JwksCache {
	return &JwksCache{
		keysets: make(map[remotehttp.FetchKey]Keyset),
	}
}

func (c *JwksCache) LoadJwksFromStores(stored []Keyset) error {
	newCache := NewCache()
	errs := make([]error, 0)

	for _, keyset := range stored {
		jwks := jose.JSONWebKeySet{}
		if err := json.Unmarshal([]byte(keyset.JwksJSON), &jwks); err != nil {
			errs = append(errs, err)
			continue
		}

		newCache.keysets[keyset.RequestKey] = keyset
	}

	c.l.Lock()
	c.keysets = newCache.keysets
	c.l.Unlock()
	return errors.Join(errs...)
}

func (c *JwksCache) GetJwks(requestKey remotehttp.FetchKey) (Keyset, bool) {
	c.l.Lock()
	defer c.l.Unlock()

	keyset, ok := c.keysets[requestKey]
	return keyset, ok
}

func buildKeyset(requestKey remotehttp.FetchKey, requestURL string, jwks jose.JSONWebKeySet) (Keyset, error) {
	serializedJwks, err := json.Marshal(jwks)
	if err != nil {
		return Keyset{}, err
	}
	return Keyset{
		RequestKey: requestKey,
		URL:        requestURL,
		FetchedAt:  time.Now(),
		JwksJSON:   string(serializedJwks),
	}, nil
}

func (c *JwksCache) putKeyset(keyset Keyset) {
	c.l.Lock()
	defer c.l.Unlock()
	c.keysets[keyset.RequestKey] = keyset
}

func (c *JwksCache) deleteJwks(requestKey remotehttp.FetchKey) bool {
	c.l.Lock()
	defer c.l.Unlock()
	_, existed := c.keysets[requestKey]
	delete(c.keysets, requestKey)
	return existed
}

func (c *JwksCache) Keys() []remotehttp.FetchKey {
	c.l.Lock()
	defer c.l.Unlock()
	keys := make([]remotehttp.FetchKey, 0, len(c.keysets))
	for k := range c.keysets {
		keys = append(keys, k)
	}
	return keys
}
