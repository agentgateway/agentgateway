package oidc

import (
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

const DefaultStorePrefix = "oidc-store"

var logger = logging.New("agentgateway/oidc")

// OidcResults stores discovered OIDC providers as a KRT-visible collection.
type OidcResults = remotecache.FetchedResults[DiscoveredProvider]

// NewFetchedResults constructs an empty OIDC fetched-result collection.
func NewFetchedResults() *OidcResults {
	return remotecache.NewFetchedResults[DiscoveredProvider]()
}

// Store bridges KRT-derived shared OIDC requests to the runtime that fetches,
// persists, and serves discovered providers to translation.
type Store struct {
	*remotecache.Store[SharedOidcRequest, DiscoveredProvider]
	// Driver is the OIDC HTTP fetch driver. Exposed so tests can swap its
	// DefaultClient for an offline transport.
	Driver *OidcDriver
}

func NewStore(requests krt.Collection[SharedOidcRequest], persistedEntries *PersistedEntries) *Store {
	fetcher, driver := NewFetcher(NewFetchedResults())
	innerStore := remotecache.NewStore(remotecache.StoreOptions[SharedOidcRequest, DiscoveredProvider]{
		Fetcher:  fetcher,
		Requests: requests,
		Logger:   logger,
		Hydrate:  persistedEntries.LoadAll,
	})

	return &Store{
		Store:  innerStore,
		Driver: driver,
	}
}

func (s *Store) RunnableName() string {
	return DefaultStorePrefix
}

var _ common.NamedRunnable = &Store{}
