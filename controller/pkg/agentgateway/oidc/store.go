package oidc

import (
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

const DefaultStorePrefix = "oidc-store"

var storeLogger = logging.New("oidc_store")

// Store bridges KRT-derived shared OIDC requests to the runtime that fetches,
// persists, and serves discovered providers to translation.
type Store struct {
	*remotecache.Store[SharedOidcRequest, DiscoveredProvider]
	cache *OidcCache
}

func NewStore(requests krt.Collection[SharedOidcRequest], persistedEntries *PersistedEntries, storePrefix string) *Store {
	cache := NewCache()
	innerStore := remotecache.NewStore(remotecache.StoreOptions[SharedOidcRequest, DiscoveredProvider]{
		Fetcher:                  NewFetcher(cache),
		Requests:                 requests,
		Logger:                   storeLogger,
		Hydrator:                 persistedEntries,
		RetireOnRequestKeyChange: true,
	})

	return &Store{
		Store: innerStore,
		cache: cache,
	}
}

// ProviderByRequestKey is the cache view used by the ConfigMap reconciler.
// Translation reads via Lookup.ResolveForOwner (KRT-backed) so it re-runs
// when ConfigMaps change.
func (s *Store) ProviderByRequestKey(requestKey remotehttp.FetchKey) (DiscoveredProvider, bool) {
	return s.cache.Get(requestKey)
}

func (s *Store) RunnableName() string {
	return DefaultStorePrefix
}

var _ common.NamedRunnable = &Store{}
