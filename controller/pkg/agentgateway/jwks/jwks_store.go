package jwks

import (
	"context"
	"sync"

	"k8s.io/apimachinery/pkg/types"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/common"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/collections"
)

// JwksStore is a top-level abstraction that relies on jwksCache and jwksFetcher to
// retrieve and keep jwks up to date.

var logger = logging.New("jwks_store")

const DefaultJwksStorePrefix = "jwks-store"
const RunnableName = "jwks-store"

var JwksConfigMapNamespacedName = func(jwksUri string) *types.NamespacedName {
	return nil
}

type JwksStore struct {
	storePrefix     string
	jwksCache       *jwksCache
	jwksFetcher     *JwksFetcher
	configMapSyncer *configMapSyncer
	jwksChanges     <-chan JwksSource
	cmNameToJwks    map[string]string
	l               sync.Mutex
}

func BuildJwksStore(ctx context.Context, cli apiclient.Client, commonCols *collections.CommonCollections, jwksChanges <-chan JwksSource, storePrefix, deploymentNamespace string) *JwksStore {
	logger.Info("creating jwks store")

	jwksCache := NewJwksCache()
	jwksStore := &JwksStore{
		storePrefix:     storePrefix,
		jwksCache:       jwksCache,
		jwksChanges:     jwksChanges,
		jwksFetcher:     NewJwksFetcher(jwksCache),
		configMapSyncer: NewConfigMapSyncer(cli, storePrefix, deploymentNamespace, commonCols.KrtOpts),
		cmNameToJwks:    make(map[string]string),
	}
	BuildJwksConfigMapNamespacedNameFunc(storePrefix, deploymentNamespace)
	return jwksStore
}

func BuildJwksConfigMapNamespacedNameFunc(storePrefix, deploymentNamespace string) {
	JwksConfigMapNamespacedName = func(jwksUri string) *types.NamespacedName {
		return &types.NamespacedName{Namespace: deploymentNamespace, Name: JwksConfigMapName(storePrefix, jwksUri)}
	}
}

func (s *JwksStore) Start(ctx context.Context) error {
	logger.Info("starting jwks store")

	storedJwks, err := s.configMapSyncer.LoadJwksFromConfigMaps(ctx)
	if err != nil {
		logger.Error("error loading jwks store from a ConfigMap", "error", err)
	}
	err = s.jwksCache.LoadJwksFromStores(storedJwks)
	if err != nil {
		logger.Error("error loading jwks store state", "error", err)
	}

	go s.jwksFetcher.Run(ctx)
	go s.updateJwksSources(ctx)

	<-ctx.Done()
	return nil
}

func (s *JwksStore) SubscribeToUpdates() chan map[string]string {
	return s.jwksFetcher.SubscribeToUpdates()
}

func (s *JwksStore) JwksByConfigMapName(cmName string) (string, string, bool) {
	s.l.Lock()
	defer s.l.Unlock()

	uri, ok := s.cmNameToJwks[cmName]
	if !ok {
		return "", "", false
	}

	jwks, ok := s.jwksCache.GetJwks(uri)
	if !ok {
		return "", "", false
	}

	return uri, jwks, true
}

func (s *JwksStore) updateJwksSources(ctx context.Context) {
	for {
		select {
		case jwksUpdate := <-s.jwksChanges:
			if jwksUpdate.Deleted {
				logger.Debug("deleting keyset", "jwks_uri", jwksUpdate.JwksURL, "config_map", JwksConfigMapName(s.storePrefix, jwksUpdate.JwksURL))
				s.jwksFetcher.RemoveKeyset(jwksUpdate)

				s.l.Lock()
				delete(s.cmNameToJwks, JwksConfigMapName(s.storePrefix, jwksUpdate.JwksURL))
				s.l.Unlock()
			} else {
				logger.Debug("updating keyset", "jwks_uri", jwksUpdate.JwksURL, "config_map", JwksConfigMapName(s.storePrefix, jwksUpdate.JwksURL))
				err := s.jwksFetcher.AddOrUpdateKeyset(jwksUpdate)
				if err != nil {
					logger.Error("error adding/updating a jwks keyset", "error", err, "uri", jwksUpdate.JwksURL)
					continue
				}

				s.l.Lock()
				s.cmNameToJwks[JwksConfigMapName(s.storePrefix, jwksUpdate.JwksURL)] = jwksUpdate.JwksURL
				s.l.Unlock()
			}
		case <-ctx.Done():
			return
		}
	}
}

// JwksStore runs on the leader only
func (r *JwksStore) NeedLeaderElection() bool {
	return true
}

var _ common.NamedRunnable = &JwksStore{}

func (r *JwksStore) RunnableName() string {
	return RunnableName
}
