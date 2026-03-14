package jwks

import (
	"context"
	"sync"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
)

var logger = logging.New("jwks_store")

const DefaultJwksStorePrefix = "jwks-store"
const RunnableName = "jwks-store"

type Store struct {
	storePrefix         string
	deploymentNamespace string
	jwksCache           *jwksCache
	jwksFetcher         *fetcher
	configMapSyncer     *configMapSyncer
	jwksChanges         <-chan JwksSource
	sourcesByOwner      map[OwnerKey]JwksSource
	ownersByRequestKey  map[RequestKey]map[OwnerKey]JwksSource
	l                   sync.Mutex
}

func NewStore(_ context.Context, cli apiclient.Client, commonCols *collections.CommonCollections, jwksChanges <-chan JwksSource, storePrefix, deploymentNamespace string) *Store {
	logger.Info("creating jwks store")

	jwksCache := newCache()
	jwksStore := &Store{
		storePrefix:         storePrefix,
		deploymentNamespace: deploymentNamespace,
		jwksCache:           jwksCache,
		jwksChanges:         jwksChanges,
		jwksFetcher:         newFetcher(jwksCache),
		configMapSyncer:     newConfigMapSyncer(cli, storePrefix, deploymentNamespace, commonCols.KrtOpts),
		sourcesByOwner:      make(map[OwnerKey]JwksSource),
		ownersByRequestKey:  make(map[RequestKey]map[OwnerKey]JwksSource),
	}
	return jwksStore
}

type storeUpdate struct {
	actions []requestAction
}

type requestAction struct {
	requestKey RequestKey
	upsert     *JwksSource
	delete     bool
}

func (s *Store) Start(ctx context.Context) error {
	logger.Info("starting jwks store")

	storedJwks, err := s.configMapSyncer.LoadJwksFromConfigMaps(ctx)
	if err != nil {
		logger.Error("error loading jwks store from a ConfigMap", "error", err)
	}
	if err := s.jwksCache.LoadJwksFromStores(storedJwks); err != nil {
		logger.Error("error loading jwks store state", "error", err)
	}

	go s.jwksFetcher.Run(ctx)
	go s.updateJwksSources(ctx)

	<-ctx.Done()
	return nil
}

func (s *Store) SubscribeToUpdates() <-chan map[RequestKey]struct{} {
	return s.jwksFetcher.SubscribeToUpdates()
}

func (s *Store) JwksByRequestKey(requestKey RequestKey) (Artifact, bool) {
	return s.jwksCache.GetJwks(requestKey)
}

func (s *Store) updateJwksSources(ctx context.Context) {
	for {
		select {
		case jwksUpdate := <-s.jwksChanges:
			var update storeUpdate
			if jwksUpdate.Deleted {
				update = s.removeOwner(jwksUpdate.OwnerKey)
			} else {
				update = s.applyOwnerUpdate(jwksUpdate)
			}

			for _, action := range update.actions {
				if action.delete {
					logger.Debug("deleting keyset", "request_key", action.requestKey, "config_map", JwksConfigMapName(s.storePrefix, action.requestKey))
					s.jwksFetcher.RemoveKeyset(action.requestKey)
					continue
				}
				if action.upsert == nil {
					continue
				}

				logger.Debug("updating keyset", "request_key", action.upsert.RequestKey, "config_map", JwksConfigMapName(s.storePrefix, action.upsert.RequestKey))
				if err := s.jwksFetcher.AddOrUpdateKeyset(*action.upsert); err != nil {
					logger.Error("error adding/updating a jwks keyset", "error", err, "request_key", action.upsert.RequestKey, "uri", action.upsert.Request.URL)
				}
			}
		case <-ctx.Done():
			return
		}
	}
}

func (s *Store) applyOwnerUpdate(source JwksSource) storeUpdate {
	s.l.Lock()
	defer s.l.Unlock()

	update := storeUpdate{actions: make([]requestAction, 0, 2)}
	if existing, ok := s.sourcesByOwner[source.OwnerKey]; ok && existing.RequestKey != source.RequestKey {
		s.deleteOwnerLocked(existing.OwnerKey, existing.RequestKey)
		update.actions = append(update.actions, s.reconcileRequestLocked(existing.RequestKey))
	}

	s.sourcesByOwner[source.OwnerKey] = source
	owners := s.ownersByRequestKey[source.RequestKey]
	if owners == nil {
		owners = make(map[OwnerKey]JwksSource)
		s.ownersByRequestKey[source.RequestKey] = owners
	}
	owners[source.OwnerKey] = source

	update.actions = append(update.actions, s.reconcileRequestLocked(source.RequestKey))

	return update
}

func (s *Store) removeOwner(ownerKey OwnerKey) storeUpdate {
	s.l.Lock()
	defer s.l.Unlock()

	existing, ok := s.sourcesByOwner[ownerKey]
	if !ok {
		return storeUpdate{}
	}

	s.deleteOwnerLocked(ownerKey, existing.RequestKey)
	return storeUpdate{actions: []requestAction{s.reconcileRequestLocked(existing.RequestKey)}}
}

func (s *Store) deleteOwnerLocked(ownerKey OwnerKey, requestKey RequestKey) {
	delete(s.sourcesByOwner, ownerKey)
	owners := s.ownersByRequestKey[requestKey]
	delete(owners, ownerKey)
	if len(owners) == 0 {
		delete(s.ownersByRequestKey, requestKey)
	}
}

func (s *Store) reconcileRequestLocked(requestKey RequestKey) requestAction {
	owners := s.ownersByRequestKey[requestKey]
	if len(owners) == 0 {
		return requestAction{requestKey: requestKey, delete: true}
	}

	shared := sharedSource(owners)
	return requestAction{requestKey: requestKey, upsert: &shared}
}

func sharedSource(owners map[OwnerKey]JwksSource) JwksSource {
	var (
		shared JwksSource
		first  = true
	)

	for _, source := range owners {
		if first {
			shared = source
			first = false
			continue
		}
		if source.TTL < shared.TTL {
			shared.TTL = source.TTL
		}
	}

	return shared
}

func (r *Store) NeedLeaderElection() bool {
	return true
}

var _ common.NamedRunnable = &Store{}

func (r *Store) RunnableName() string {
	return RunnableName
}
