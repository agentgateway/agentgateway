package jwks

import (
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

const DefaultJwksStorePrefix = "jwks-store"
const RunnableName = "jwks-store"

var storeLogger = logging.New("jwks_store")

// Store bridges KRT-derived shared JWKS requests to the runtime that fetches,
// persists, and serves keysets to translation.
type Store struct {
	*remotecache.Store[SharedJwksRequest, Keyset]
	cache *JwksCache
}

func NewStore(requests krt.Collection[SharedJwksRequest], persistedEntries *PersistedEntries, storePrefix string) *Store {
	cache := NewCache()
	innerStore := remotecache.NewStore(remotecache.StoreOptions[SharedJwksRequest, Keyset]{
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

func (s *Store) JwksByRequestKey(requestKey remotehttp.FetchKey) (Keyset, bool) {
	return s.cache.Get(requestKey)
}

func (s *Store) RunnableName() string {
	return RunnableName
}

var _ common.NamedRunnable = &Store{}
