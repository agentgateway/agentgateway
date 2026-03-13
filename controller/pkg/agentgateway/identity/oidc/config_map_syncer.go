package oidc

import (
	"context"
	"errors"
	"maps"

	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

type configMapSyncer struct {
	cmCollection krt.Collection[*corev1.ConfigMap]
}

func NewConfigMapSyncer(
	client apiclient.Client,
	storePrefix string,
	deploymentNamespace string,
	krtOptions krtutil.KrtOptions,
) *configMapSyncer {
	cmCollection := krt.NewFilteredInformer[*corev1.ConfigMap](
		client,
		kclient.Filter{
			ObjectFilter:  client.ObjectFilter(),
			LabelSelector: StoreLabelSelector(storePrefix),
		},
		krtOptions.ToOptions("oidc_provider_store/ConfigMaps")...,
	)

	return &configMapSyncer{
		cmCollection: cmCollection,
	}
}

func (cs *configMapSyncer) LoadProvidersFromConfigMaps(
	ctx context.Context,
) (map[string]StoredProvider, error) {
	log := logger.With("component", "config_map_syncer")

	allPersistedProviders := cs.cmCollection.List()
	if len(allPersistedProviders) == 0 {
		return nil, nil
	}

	errs := make([]error, 0)
	providers := make(map[string]StoredProvider)
	for _, cm := range allPersistedProviders {
		provider, err := ProviderFromConfigMap(cm)
		if err != nil {
			log.Error("error deserializing oidc provider ConfigMap", "error", err, "ConfigMap", cm.Name)
			errs = append(errs, err)
			continue
		}
		maps.Copy(providers, map[string]StoredProvider{provider.ResourceKey: provider})
	}

	return providers, errors.Join(errs...)
}
