package plugins

import (
	"testing"

	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

type stubOIDCResolver struct {
	resolved *resolvedOIDCProvider
	err      error
}

func (s stubOIDCResolver) Resolve(
	_ PolicyCtx,
	_,
	_,
	_ string,
	_ *gwv1.BackendObjectReference,
) (*resolvedOIDCProvider, error) {
	return s.resolved, s.err
}

func withStubOIDCResolver(t *testing.T, resolved *resolvedOIDCProvider, err error) {
	t.Helper()
	prev := oidcResolverFactory
	oidcResolverFactory = func() oidcResolver {
		return stubOIDCResolver{resolved: resolved, err: err}
	}
	t.Cleanup(func() {
		oidcResolverFactory = prev
	})
}
