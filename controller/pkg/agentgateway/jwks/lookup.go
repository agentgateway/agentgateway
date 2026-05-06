package jwks

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
)

type Lookup interface {
	InlineForOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (string, error)
}

type lookup struct {
	resolver  Resolver
	persisted *PersistedEntries
}

func NewLookup(persisted *PersistedEntries, resolver Resolver) Lookup {
	return &lookup{
		resolver:  resolver,
		persisted: persisted,
	}
}

func (l *lookup) InlineForOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (string, error) {
	if l.persisted == nil {
		return "", fmt.Errorf("jwks persisted cache is not configured")
	}

	resolved, err := l.resolver.ResolveOwner(krtctx, owner)
	if err != nil {
		return "", err
	}

	keyset, ok := l.persisted.CanonicalGet(krtctx, resolved.Target.Target.Key())
	if !ok {
		return "", fmt.Errorf("jwks keyset for %q isn't available (not yet fetched or fetch failed)", resolved.Target.Target.URL)
	}
	return keyset.JwksJSON, nil
}
