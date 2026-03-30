package jwks

import (
	"encoding/json"
	"errors"
	"sync"
	"time"

	"github.com/go-jose/go-jose/v4"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// jwksCache stores fetched JWKS keysets by request key.
type jwksCache struct {
	l       sync.Mutex
	keysets map[remotehttp.FetchKey]Keyset
}

func newCache() *jwksCache {
	return &jwksCache{
		keysets: make(map[remotehttp.FetchKey]Keyset),
	}
}

func (c *jwksCache) LoadJwksFromStores(stored []Keyset) error {
	newCache := newCache()
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

func (c *jwksCache) GetJwks(requestKey remotehttp.FetchKey) (Keyset, bool) {
	c.l.Lock()
	defer c.l.Unlock()

	keyset, ok := c.keysets[requestKey]
	return keyset, ok
}

func (c *jwksCache) addJwks(requestKey remotehttp.FetchKey, requestURL string, jwks jose.JSONWebKeySet) error {
	serializedJwks, err := json.Marshal(jwks)
	if err != nil {
		return err
	}

	c.l.Lock()
	defer c.l.Unlock()

	keyset := Keyset{
		RequestKey: requestKey,
		URL:        requestURL,
		FetchedAt:  time.Now(),
		JwksJSON:   string(serializedJwks),
	}
	c.keysets[requestKey] = keyset
	return nil
}

func (c *jwksCache) deleteJwks(requestKey remotehttp.FetchKey) {
	c.l.Lock()
	delete(c.keysets, requestKey)
	c.l.Unlock()
}
