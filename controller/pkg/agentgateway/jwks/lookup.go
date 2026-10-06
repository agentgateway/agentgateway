package jwks

import (
	"errors"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
)

type Lookup interface {
	InlineForOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (string, error)
}

type lookup struct {
	owners krt.Collection[ResolvedOwner]
	cache  *keysetCache
}

func NewLookup(persisted *PersistedEntries, owners krt.Collection[ResolvedOwner]) Lookup {
	return &lookup{
		owners: owners,
		cache:  newKeysetCache(persisted),
	}
}

func (l *lookup) InlineForOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (string, error) {
	if l.cache == nil {
		return "", fmt.Errorf("jwks persisted cache is not configured")
	}

	resolved := krt.FetchOne(krtctx, l.owners, krt.FilterKey(owner.ResourceName()))
	if resolved == nil {
		return "", fmt.Errorf("jwks resolution for %q isn't available", owner.ResourceName())
	}
	if resolved.Error != "" {
		return "", errors.New(resolved.Error)
	}

	keyset, ok := l.cache.Get(krtctx, resolved.Source.RequestKey)
	if !ok {
		return "", fmt.Errorf("jwks keyset for %q isn't available (not yet fetched or fetch failed)", resolved.Source.Target.URL)
	}
	return keyset.JwksJSON, nil
}
