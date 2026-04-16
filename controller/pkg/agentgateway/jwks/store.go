package jwks

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

var logger = logging.New("jwks_store")

const DefaultJwksStorePrefix = "jwks-store"
const RunnableName = "jwks-store"

// Store bridges KRT-derived shared JWKS requests to the runtime that fetches,
// persists, and serves keysets to translation.
//
// For JWT auth, JWKS is the persisted last-known-good boundary for explicit
// remote key fetches.
type Store struct {
	storePrefix      string
	jwksCache        *JwksCache
	jwksFetcher      *Fetcher
	persistedKeysets *persistedKeysetReader
	requests         krt.Collection[SharedJwksRequest]
	ready            chan struct{}
}

func NewStore(requests krt.Collection[SharedJwksRequest], persistedKeysets *PersistedEntries, storePrefix string) *Store {
	logger.Info("creating jwks store")

	jwksCache := NewCache()
	return &Store{
		storePrefix:      storePrefix,
		jwksCache:        jwksCache,
		requests:         requests,
		jwksFetcher:      NewFetcher(jwksCache),
		persistedKeysets: newPersistedKeysetReader(persistedKeysets),
		ready:            make(chan struct{}),
	}
}

func (s *Store) Start(ctx context.Context) error {
	logger.Info("starting jwks store")

	if s.persistedKeysets == nil {
		return fmt.Errorf("jwks persisted keyset reader is not configured")
	}

	storedJwks, err := s.persistedKeysets.LoadPersistedKeysets(ctx)
	if err != nil {
		logger.Error("error loading jwks store from a ConfigMap", "error", err)
	}
	if err := s.jwksCache.LoadJwksFromStores(storedJwks); err != nil {
		logger.Error("error loading jwks store state", "error", err)
	}

	registration := s.requests.Register(func(event krt.Event[SharedJwksRequest]) {
		switch event.Event {
		case controllers.EventAdd, controllers.EventUpdate:
			if event.New == nil {
				return
			}

			request := event.New.JwksSource()
			logger.Debug("updating keyset", "request_key", request.RequestKey, "config_map", JwksConfigMapName(s.storePrefix, request.RequestKey))
			if err := s.jwksFetcher.AddOrUpdateKeyset(request); err != nil {
				logger.Error("error adding/updating a jwks keyset", "error", err, "request_key", request.RequestKey, "uri", request.Target.URL)
			}
		case controllers.EventDelete:
			if event.Old == nil {
				return
			}

			logger.Debug("deleting keyset", "request_key", event.Old.RequestKey, "config_map", JwksConfigMapName(s.storePrefix, event.Old.RequestKey))
			s.jwksFetcher.RemoveKeyset(event.Old.RequestKey)
		}
	})
	defer registration.UnregisterHandler()

	go s.jwksFetcher.Run(ctx)

	if !registration.WaitUntilSynced(ctx.Done()) {
		return nil
	}
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
	return s.jwksFetcher.SubscribeToUpdates()
}

func (s *Store) JwksByRequestKey(requestKey remotehttp.FetchKey) (Keyset, bool) {
	return s.jwksCache.GetJwks(requestKey)
}

func (r *Store) NeedLeaderElection() bool {
	return true
}

var _ common.NamedRunnable = &Store{}

func (r *Store) RunnableName() string {
	return RunnableName
}
