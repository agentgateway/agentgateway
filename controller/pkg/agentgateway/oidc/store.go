package oidc

import (
	"context"
	"fmt"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

var logger = logging.New("oidc_store")

const DefaultStorePrefix = "oidc-store"
const RunnableName = "oidc-store"

// Store bridges KRT-derived shared OIDC requests to the runtime that fetches,
// persists, and serves discovered providers to translation.
//
// For OIDC auth, the pre-fetched discovery document is the boundary for explicit
// remote discovery fetches.
type Store struct {
	storePrefix      string
	oidcCache        *OidcCache
	oidcFetcher      *Fetcher
	persistedReaders *persistedProviderReader
	requests         krt.Collection[SharedOidcRequest]
	ready            chan struct{}
}

func NewStore(requests krt.Collection[SharedOidcRequest], persistedEntries *PersistedEntries, storePrefix string) *Store {
	logger.Info("creating oidc store")

	oidcCache := NewCache()
	return &Store{
		storePrefix:      storePrefix,
		oidcCache:        oidcCache,
		requests:         requests,
		oidcFetcher:      NewFetcher(oidcCache),
		persistedReaders: newPersistedProviderReader(persistedEntries),
		ready:            make(chan struct{}),
	}
}

func (s *Store) Start(ctx context.Context) error {
	logger.Info("starting oidc store")

	if s.persistedReaders == nil {
		return fmt.Errorf("oidc persisted provider reader is not configured")
	}

	storedProviders, err := s.persistedReaders.LoadPersistedProviders(ctx)
	if err != nil {
		logger.Error("error loading oidc store from a ConfigMap", "error", err)
	}
	if err := s.oidcCache.LoadProvidersFromStores(storedProviders); err != nil {
		logger.Error("error loading oidc store state", "error", err)
	}

	registration := s.requests.Register(func(event krt.Event[SharedOidcRequest]) {
		switch event.Event {
		case controllers.EventAdd, controllers.EventUpdate:
			if event.New == nil {
				return
			}

			request := event.New.OidcSource()
			logger.Debug("updating provider", "request_key", request.RequestKey, "config_map", OidcConfigMapName(s.storePrefix, request.RequestKey))
			if err := s.oidcFetcher.AddOrUpdateProvider(request); err != nil {
				logger.Error("error adding/updating an oidc provider", "error", err, "request_key", request.RequestKey, "issuer", request.Target.URL)
			}
		case controllers.EventDelete:
			if event.Old == nil {
				return
			}

			logger.Debug("deleting provider", "request_key", event.Old.RequestKey, "config_map", OidcConfigMapName(s.storePrefix, event.Old.RequestKey))
			s.oidcFetcher.RemoveOidc(event.Old.RequestKey)
		}
	})
	defer registration.UnregisterHandler()

	go s.oidcFetcher.Run(ctx)

	if !registration.WaitUntilSynced(ctx.Done()) {
		return nil
	}

	// Sweep orphans before flipping HasSynced so plugins never observe stale
	// providers whose owning policies were removed while the controller was down.
	s.oidcFetcher.SweepOrphans()

	close(s.ready)

	<-ctx.Done()
	return nil
}

func (s *Store) HasSynced() bool {
	select {
	case <-s.ready:
		return true
	default:
		return false
	}
}

func (s *Store) SubscribeToUpdates() <-chan sets.Set[remotehttp.FetchKey] {
	return s.oidcFetcher.SubscribeToUpdates()
}

func (s *Store) ProviderByRequestKey(requestKey remotehttp.FetchKey) (DiscoveredProvider, bool) {
	return s.oidcCache.GetProvider(requestKey)
}

func (s *Store) NeedLeaderElection() bool {
	return true
}

var _ common.NamedRunnable = &Store{}

func (s *Store) RunnableName() string {
	return RunnableName
}
