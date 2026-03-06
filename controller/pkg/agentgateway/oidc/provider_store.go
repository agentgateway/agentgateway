package oidc

import (
	"context"
	"sync"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
)

var logger = logging.New("oidc_provider_store")

const RunnableName = "oidc-provider-store"

type Store struct {
	storePrefix     string
	cache           *providerCache
	fetcher         *ProviderFetcher
	configMapSyncer *configMapSyncer
	sourceChanges   <-chan ProviderSource
	cmNameToKey     map[string]string
	sourceOwners    map[string]map[string]ProviderSource
	l               sync.Mutex
}

func BuildProviderStore(
	ctx context.Context,
	cli apiclient.Client,
	commonCols *collections.CommonCollections,
	sourceChanges <-chan ProviderSource,
	storePrefix string,
	deploymentNamespace string,
) *Store {
	logger.Info("creating oidc provider store")

	cache := NewProviderCache()
	store := &Store{
		storePrefix:     storePrefix,
		cache:           cache,
		sourceChanges:   sourceChanges,
		fetcher:         NewProviderFetcher(cache),
		configMapSyncer: NewConfigMapSyncer(cli, storePrefix, deploymentNamespace, commonCols.KrtOpts),
		cmNameToKey:     make(map[string]string),
		sourceOwners:    make(map[string]map[string]ProviderSource),
	}
	BuildProviderConfigMapNamespacedNameFunc(storePrefix, deploymentNamespace)
	return store
}

func (s *Store) Start(ctx context.Context) error {
	logger.Info("starting oidc provider store")

	storedProviders, err := s.configMapSyncer.LoadProvidersFromConfigMaps(ctx)
	if err != nil {
		logger.Error("error loading oidc provider store from ConfigMaps", "error", err)
	}
	s.cache.LoadProvidersFromStores(storedProviders)

	go s.fetcher.Run(ctx)
	go s.updateSources(ctx)

	<-ctx.Done()
	return nil
}

func (s *Store) SubscribeToUpdates() chan map[string]StoredProvider {
	return s.fetcher.SubscribeToUpdates()
}

func (s *Store) ProviderByConfigMapName(cmName string) (string, StoredProvider, bool) {
	s.l.Lock()
	defer s.l.Unlock()

	key, ok := s.cmNameToKey[cmName]
	if !ok {
		return "", StoredProvider{}, false
	}

	provider, ok := s.cache.GetProvider(key)
	if !ok {
		return "", StoredProvider{}, false
	}
	return key, provider, true
}

func (s *Store) updateSources(ctx context.Context) {
	for {
		select {
		case source := <-s.sourceChanges:
			if err := s.applySourceChange(source); err != nil {
				logger.Error("error applying oidc provider source change", "error", err, "issuer", source.Issuer)
			}
		case <-ctx.Done():
			return
		}
	}
}

func (s *Store) applySourceChange(source ProviderSource) error {
	s.l.Lock()
	defer s.l.Unlock()

	ownerKey := source.OwnerKey
	if ownerKey == "" {
		ownerKey = source.ResourceName()
	}

	if source.Deleted {
		logger.Debug(
			"deleting oidc provider source owner",
			"issuer", source.Issuer,
			"config_map", ProviderConfigMapName(s.storePrefix, source.ResourceKey),
			"owner", ownerKey,
		)

		owners, ok := s.sourceOwners[source.ResourceKey]
		if !ok {
			return nil
		}
		delete(owners, ownerKey)
		if len(owners) == 0 {
			// Only remove the shared fetched source when the last owner disappears.
			delete(s.sourceOwners, source.ResourceKey)
			s.fetcher.RemoveSource(source)
			delete(s.cmNameToKey, ProviderConfigMapName(s.storePrefix, source.ResourceKey))
			return nil
		}

		for _, remaining := range owners {
			if !remaining.Equivalent(source) {
				return s.fetcher.AddOrUpdateSource(remaining)
			}
			break
		}
		return nil
	}

	logger.Debug(
		"updating oidc provider source owner",
		"issuer", source.Issuer,
		"config_map", ProviderConfigMapName(s.storePrefix, source.ResourceKey),
		"owner", ownerKey,
	)

	owners, ok := s.sourceOwners[source.ResourceKey]
	if !ok {
		owners = make(map[string]ProviderSource)
		s.sourceOwners[source.ResourceKey] = owners
	}

	var representative ProviderSource
	haveRepresentative := false
	for _, existing := range owners {
		representative = existing
		haveRepresentative = true
		break
	}

	owners[ownerKey] = source
	s.cmNameToKey[ProviderConfigMapName(s.storePrefix, source.ResourceKey)] = source.ResourceKey

	// The store owns dedupe; policy controllers emit owner-scoped sources.
	if haveRepresentative && representative.Equivalent(source) {
		return nil
	}
	return s.fetcher.AddOrUpdateSource(source)
}

func (r *Store) NeedLeaderElection() bool {
	return true
}

func (r *Store) RunnableName() string {
	return RunnableName
}

var _ common.NamedRunnable = &Store{}
var _ interface{ Start(context.Context) error } = &Store{}
var _ interface{ NeedLeaderElection() bool } = &Store{}
var _ interface {
	ProviderByConfigMapName(string) (string, StoredProvider, bool)
} = &Store{}
