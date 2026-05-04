package oidc

import (
	"encoding/json"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// oidcConfigMapKey is the key inside an OIDC-store ConfigMap's Data map that
// holds the serialized DiscoveredProvider.
const oidcConfigMapKey = "oidc-store"

// observabilityName names the OIDC persisted-cache subsystem in KRT collection
// metric labels. Stable across releases for metric continuity.
const observabilityName = "persisted_oidc"

// PersistedEntry is the parsed persisted OIDC record view for a single
// ConfigMap.
type PersistedEntry = remotecache.Entry[DiscoveredProvider]

// PersistedEntries is the OIDC-specific KRT view over OIDC-store ConfigMaps.
type PersistedEntries = remotecache.Entries[DiscoveredProvider]

// OidcCodec returns the persistedcache codec for OIDC discovered providers.
func OidcCodec() remotecache.Codec[DiscoveredProvider] {
	return remotecache.Codec[DiscoveredProvider]{
		DataKey:           oidcConfigMapKey,
		ObservabilityName: observabilityName,
		Parse:             ProviderFromConfigMap,
		Serialize:         SetProviderInConfigMap,
		Normalize:         normalizePersistedProvider,
	}
}

// NewPersistedEntries constructs the KRT-backed OIDC persistence layer.
func NewPersistedEntries(client apiclient.Client, krtOptions krtutil.KrtOptions, storePrefix, deploymentNamespace string) *PersistedEntries {
	return remotecache.New(OidcCodec(), client, krtOptions, storePrefix, deploymentNamespace)
}

// NewPersistedEntriesFromCollection constructs a PersistedEntries from an
// existing ConfigMap collection. Useful for testing with static collections.
func NewPersistedEntriesFromCollection(configMaps krt.Collection[*corev1.ConfigMap], storePrefix, deploymentNamespace string) *PersistedEntries {
	return remotecache.NewFromCollection(OidcCodec(), configMaps, storePrefix, deploymentNamespace)
}

// ProviderFromConfigMap parses a DiscoveredProvider from a ConfigMap.
func ProviderFromConfigMap(cm *corev1.ConfigMap) (DiscoveredProvider, error) {
	data, ok := cm.Data[oidcConfigMapKey]
	if !ok {
		return DiscoveredProvider{}, fmt.Errorf("OIDC provider ConfigMap %s/%s missing data key %q", cm.Namespace, cm.Name, oidcConfigMapKey)
	}

	var provider DiscoveredProvider
	if err := json.Unmarshal([]byte(data), &provider); err != nil {
		return DiscoveredProvider{}, fmt.Errorf("failed to unmarshal OIDC provider from ConfigMap %s/%s: %w", cm.Namespace, cm.Name, err)
	}
	if provider.RequestKey == "" {
		return DiscoveredProvider{}, fmt.Errorf("OIDC provider ConfigMap %s/%s has empty requestKey", cm.Namespace, cm.Name)
	}
	if provider.JwksInline == "" {
		return DiscoveredProvider{}, fmt.Errorf("OIDC provider ConfigMap %s/%s has empty jwksInline", cm.Namespace, cm.Name)
	}
	return provider, nil
}

// SetProviderInConfigMap serializes a DiscoveredProvider into a ConfigMap.
func SetProviderInConfigMap(cm *corev1.ConfigMap, provider DiscoveredProvider) error {
	b, err := json.Marshal(provider)
	if err != nil {
		return err
	}
	if cm.Data == nil {
		cm.Data = make(map[string]string)
	}
	cm.Data[oidcConfigMapKey] = string(b)
	return nil
}

// normalizePersistedProvider repairs a parsed DiscoveredProvider's RequestKey
// when its derivation logic has evolved. If the ConfigMap's name matches the
// canonical hash for the issuer-derived request key, rewrite RequestKey to
// the current derivation so lookups by it succeed after upgrade.
func normalizePersistedProvider(storePrefix, configMapName string, provider DiscoveredProvider) DiscoveredProvider {
	if provider.IssuerURL == "" {
		return provider
	}

	// Derive the canonical request key for this issuer. OIDC supports only
	// direct issuer discovery URLs at present.
	discoveryURL, err := OidcDiscoveryURL(provider.IssuerURL)
	if err != nil {
		return provider
	}
	requestKeyFromURL := oidcRequestKey(remotehttp.FetchTarget{URL: discoveryURL}, provider.IssuerURL)

	if remotecache.ConfigMapName(storePrefix, requestKeyFromURL) == configMapName {
		provider.RequestKey = requestKeyFromURL
	}

	return provider
}
