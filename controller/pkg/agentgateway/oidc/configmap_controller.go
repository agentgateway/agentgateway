package oidc

import (
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

// ConfigMapController synchronizes persisted OIDC providers to ConfigMaps.
type ConfigMapController struct {
	*remotecache.ConfigMapController[DiscoveredProvider, PersistedEntry]
}

// ConfigMapControllerOptions configures NewConfigMapController.
type ConfigMapControllerOptions struct {
	APIClient           apiclient.Client
	StorePrefix         string
	DeploymentNamespace string
	Store               *Store
	PersistedEntries    *PersistedEntries
}

func NewConfigMapController(opts ConfigMapControllerOptions) *ConfigMapController {
	logger := logging.New("oidc_store_config_map_controller")
	logger.Info("creating oidc store ConfigMap controller")

	cacheOpts := remotecache.ConfigMapControllerOptions[DiscoveredProvider, PersistedEntry]{
		ApiClient:            opts.APIClient,
		DeploymentNamespace:  opts.DeploymentNamespace,
		ControllerName:       "OidcStoreConfigMapController",
		Updates:              opts.Store.SubscribeToUpdates(),
		Entries:              opts.PersistedEntries.Collection(),
		EntriesForRequestKey: opts.PersistedEntries.EntriesForRequestKey,
		Lookup:               opts.Store.ProviderByRequestKey,
		Serialize:            SetProviderInConfigMap,
		NameFunc:             opts.PersistedEntries.ConfigMapName,
		LabelFunc:            opts.PersistedEntries.ConfigMapLabels,
		LabelSelector:        opts.PersistedEntries.LabelSelector,
		StoreHasSynced:       opts.Store.HasSynced,
		Logger:               logger,
	}

	return &ConfigMapController{
		ConfigMapController: remotecache.NewConfigMapController(cacheOpts),
	}
}
