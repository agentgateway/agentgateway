package jwks

import (
	"encoding/json"
	"errors"
	"sync"
	"time"

	"github.com/go-jose/go-jose/v4"
)

// jwksCache stores fetched JWKS artifacts by request key.
type jwksCache struct {
	l         sync.Mutex
	artifacts map[RequestKey]Artifact
}

func newCache() *jwksCache {
	return &jwksCache{
		artifacts: make(map[RequestKey]Artifact),
	}
}

func (c *jwksCache) LoadJwksFromStores(stored []Artifact) error {
	newCache := newCache()
	errs := make([]error, 0)

	for _, artifact := range stored {
		jwks := jose.JSONWebKeySet{}
		if err := json.Unmarshal([]byte(artifact.JwksJSON), &jwks); err != nil {
			errs = append(errs, err)
			continue
		}

		newCache.artifacts[artifact.RequestKey] = artifact
	}

	c.l.Lock()
	c.artifacts = newCache.artifacts
	c.l.Unlock()
	return errors.Join(errs...)
}

func (c *jwksCache) GetJwks(requestKey RequestKey) (Artifact, bool) {
	c.l.Lock()
	defer c.l.Unlock()

	artifact, ok := c.artifacts[requestKey]
	return artifact, ok
}

func (c *jwksCache) addJwks(requestKey RequestKey, requestURL string, jwks jose.JSONWebKeySet) error {
	serializedJwks, err := json.Marshal(jwks)
	if err != nil {
		return err
	}

	c.l.Lock()
	defer c.l.Unlock()

	artifact := Artifact{
		RequestKey: requestKey,
		URL:        requestURL,
		FetchedAt:  time.Now(),
		JwksJSON:   string(serializedJwks),
	}
	c.artifacts[requestKey] = artifact
	return nil
}

func (c *jwksCache) deleteJwks(requestKey RequestKey) {
	c.l.Lock()
	delete(c.artifacts, requestKey)
	c.l.Unlock()
}
