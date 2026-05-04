package jwks

import (
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

// ConfigMapController synchronizes persisted JWKS keysets to ConfigMaps.
type ConfigMapController struct {
	*remotecache.ConfigMapController[Keyset, PersistedEntry]
}

func NewConfigMapController(apiClient apiclient.Client, storePrefix, deploymentNamespace string, store *Store, persistedEntries *PersistedEntries) *ConfigMapController {
	logger := logging.New("jwks_store_config_map_controller")
	logger.Info("creating jwks store ConfigMap controller")

	opts := remotecache.ConfigMapControllerOptions[Keyset, PersistedEntry]{
		ApiClient:            apiClient,
		DeploymentNamespace:  deploymentNamespace,
		ControllerName:       "JwksStoreConfigMapController",
		Updates:              store.SubscribeToUpdates(),
		Entries:              persistedEntries.Collection(),
		EntriesForRequestKey: persistedEntries.EntriesForRequestKey,
		Lookup:               store.JwksByRequestKey,
		Serialize:            SetJwksInConfigMap,
		NameFunc:             persistedEntries.ConfigMapName,
		LabelFunc:            persistedEntries.ConfigMapLabels,
		LabelSelector:        persistedEntries.LabelSelector,
		StoreHasSynced:       store.HasSynced,
		Logger:               logger,
	}

	return &ConfigMapController{
		ConfigMapController: remotecache.NewConfigMapController(opts),
	}
}
