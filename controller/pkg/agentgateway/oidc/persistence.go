package oidc

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"

	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/log"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

const (
	// oidcConfigMapKey is the key in the ConfigMap data map that holds the serialized DiscoveredProvider.
	oidcConfigMapKey = "oidc-store"
	// oidcStoreComponentLabel is the label key used to identify OIDC store ConfigMaps.
	oidcStoreComponentLabel = "app.kubernetes.io/component"
)

// OidcStoreLabelSelector returns a label selector string for ConfigMaps
// belonging to the OIDC store with the given prefix.
func OidcStoreLabelSelector(storePrefix string) string {
	return oidcStoreComponentLabel + "=" + storePrefix
}

// OidcStoreConfigMapLabel returns the label map for OIDC store ConfigMaps.
func OidcStoreConfigMapLabel(storePrefix string) map[string]string {
	return map[string]string{oidcStoreComponentLabel: storePrefix}
}

// PersistedEntry is the parsed persisted OIDC artifact view for a single
// ConfigMap. It preserves the backing ConfigMap identity so callers can reason
// about canonical and legacy artifacts for the same request key.
type PersistedEntry struct {
	NamespacedName types.NamespacedName
	Provider       *DiscoveredProvider
	ParseError     string
}

func (e PersistedEntry) ResourceName() string {
	return e.NamespacedName.String()
}

func (e PersistedEntry) Equals(other PersistedEntry) bool {
	return e.NamespacedName == other.NamespacedName &&
		e.ParseError == other.ParseError &&
		providersEqual(e.Provider, other.Provider)
}

func (e PersistedEntry) RequestKey() (remotehttp.FetchKey, bool) {
	if e.Provider == nil {
		return "", false
	}
	return e.Provider.RequestKey, true
}

// PersistedEntries is the KRT-backed collection of persisted OIDC providers
// loaded from ConfigMaps in the deployment namespace.
type PersistedEntries struct {
	storePrefix  string
	entries      krt.Collection[PersistedEntry]
	byRequestKey krt.Index[remotehttp.FetchKey, PersistedEntry]
}

// providerCache provides canonical lookup semantics over the shared persisted
// OIDC collection. Inline OIDC resolution only trusts the canonical ConfigMap name.
type providerCache struct {
	persisted *PersistedEntries
}

// persistedProviderReader provides hydration semantics over the shared
// persisted OIDC collection. Startup loading may fall back to legacy/non-canonical
// artifacts while migration cleanup converges persisted state.
type persistedProviderReader struct {
	persisted *PersistedEntries
}

// NewPersistedEntries constructs a PersistedEntries collection by watching
// ConfigMaps labeled with OidcStoreLabelSelector in the deployment namespace.
func NewPersistedEntries(client apiclient.Client, krtOptions krtutil.KrtOptions, storePrefix, deploymentNamespace string) *PersistedEntries {
	configMaps := krt.NewFilteredInformer[*corev1.ConfigMap](client, kclient.Filter{
		ObjectFilter:  client.ObjectFilter(),
		Namespace:     deploymentNamespace,
		LabelSelector: OidcStoreLabelSelector(storePrefix),
	}, krtOptions.ToOptions("persisted_oidc/ConfigMaps")...)

	return NewPersistedEntriesFromCollection(configMaps, storePrefix, deploymentNamespace)
}

// NewPersistedEntriesFromCollection constructs a PersistedEntries from an
// existing ConfigMap collection. Useful for testing with static collections.
func NewPersistedEntriesFromCollection(configMaps krt.Collection[*corev1.ConfigMap], storePrefix, deploymentNamespace string) *PersistedEntries {
	entries := krt.NewCollection(configMaps, func(krtctx krt.HandlerContext, cm *corev1.ConfigMap) *PersistedEntry {
		if cm == nil {
			return nil
		}
		if cm.Namespace != deploymentNamespace {
			return nil
		}
		if cm.Labels[oidcStoreComponentLabel] != storePrefix {
			return nil
		}

		entry := PersistedEntry{
			NamespacedName: types.NamespacedName{
				Namespace: cm.Namespace,
				Name:      cm.Name,
			},
		}
		provider, err := ProviderFromConfigMap(cm)
		if err != nil {
			entry.ParseError = err.Error()
			return &entry
		}
		provider = normalizePersistedProvider(storePrefix, cm.Name, provider)
		entry.Provider = &provider
		return &entry
	})

	return &PersistedEntries{
		storePrefix: storePrefix,
		entries:     entries,
		byRequestKey: krt.NewIndex(entries, "persisted-oidc-request-key", func(entry PersistedEntry) []remotehttp.FetchKey {
			requestKey, ok := entry.RequestKey()
			if !ok {
				return nil
			}
			return []remotehttp.FetchKey{requestKey}
		}),
	}
}

func newProviderCache(persisted *PersistedEntries) *providerCache {
	if persisted == nil {
		return nil
	}
	return &providerCache{persisted: persisted}
}

func newPersistedProviderReader(persisted *PersistedEntries) *persistedProviderReader {
	if persisted == nil {
		return nil
	}
	return &persistedProviderReader{persisted: persisted}
}

// ProviderFromConfigMap parses a DiscoveredProvider from a ConfigMap.
func ProviderFromConfigMap(cm *corev1.ConfigMap) (DiscoveredProvider, error) {
	data := cm.Data[oidcConfigMapKey]

	var provider DiscoveredProvider
	if err := json.Unmarshal([]byte(data), &provider); err == nil && provider.RequestKey != "" {
		return provider, nil
	}

	return DiscoveredProvider{}, fmt.Errorf("failed to unmarshal OIDC provider from ConfigMap %s/%s", cm.Namespace, cm.Name)
}

// RequestKeyFromConfigMap extracts the request key from a ConfigMap.
func RequestKeyFromConfigMap(cm *corev1.ConfigMap) (remotehttp.FetchKey, error) {
	provider, err := ProviderFromConfigMap(cm)
	if err != nil {
		return "", err
	}
	return provider.RequestKey, nil
}

// OidcConfigMapName returns the canonical ConfigMap name for a given store prefix and request key.
func OidcConfigMapName(storePrefix string, requestKey remotehttp.FetchKey) string {
	sum := sha256.Sum256([]byte(requestKey))
	return fmt.Sprintf("%s-%s", storePrefix, hex.EncodeToString(sum[:]))
}

// OidcConfigMapNamespacedName returns the canonical ConfigMap namespaced name.
func OidcConfigMapNamespacedName(storePrefix, namespace string, requestKey remotehttp.FetchKey) types.NamespacedName {
	return types.NamespacedName{
		Namespace: namespace,
		Name:      OidcConfigMapName(storePrefix, requestKey),
	}
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

func (ps *PersistedEntries) entriesForRequestKey(requestKey remotehttp.FetchKey) []PersistedEntry {
	return ps.byRequestKey.Lookup(requestKey)
}

func (c *providerCache) Get(krtctx krt.HandlerContext, requestKey remotehttp.FetchKey) (DiscoveredProvider, bool) {
	if c == nil || c.persisted == nil {
		return DiscoveredProvider{}, false
	}

	entries := krt.Fetch(krtctx, c.persisted.entries, krt.FilterIndex(c.persisted.byRequestKey, requestKey))
	canonicalName := OidcConfigMapName(c.persisted.storePrefix, requestKey)
	for _, entry := range entries {
		if entry.Provider == nil {
			continue
		}
		if entry.NamespacedName.Name == canonicalName {
			return *entry.Provider, true
		}
	}
	return DiscoveredProvider{}, false
}

func (r *persistedProviderReader) LoadPersistedProviders(ctx context.Context) ([]DiscoveredProvider, error) {
	if r == nil || r.persisted == nil {
		return nil, nil
	}

	log := log.FromContext(ctx)

	kube.WaitForCacheSync("OIDC persisted providers", ctx.Done(), r.persisted.entries.HasSynced)

	allPersistedEntries := r.persisted.entries.List()
	if len(allPersistedEntries) == 0 {
		return nil, nil
	}

	errs := make([]error, 0)
	entriesByRequestKey := make(map[remotehttp.FetchKey][]PersistedEntry)
	for _, entry := range allPersistedEntries {
		requestKey, ok := entry.RequestKey()
		if !ok {
			err := fmt.Errorf("error deserializing OIDC ConfigMap %s: %s", entry.NamespacedName.String(), entry.ParseError)
			log.Error(err, "error deserializing OIDC ConfigMap", "ConfigMap", entry.NamespacedName.String())
			errs = append(errs, err)
			continue
		}
		entriesByRequestKey[requestKey] = append(entriesByRequestKey[requestKey], entry)
	}

	providers := make([]DiscoveredProvider, 0, len(entriesByRequestKey))
	for requestKey, entries := range entriesByRequestKey {
		provider, ok := r.hydrationProvider(requestKey, entries)
		if !ok {
			continue
		}
		providers = append(providers, provider)
	}

	return providers, errors.Join(errs...)
}

func (r *persistedProviderReader) hydrationProvider(requestKey remotehttp.FetchKey, entries []PersistedEntry) (DiscoveredProvider, bool) {
	best := r.bestHydrationEntry(requestKey, entries)
	if best == nil || best.Provider == nil {
		return DiscoveredProvider{}, false
	}
	return *best.Provider, true
}

func (r *persistedProviderReader) bestHydrationEntry(requestKey remotehttp.FetchKey, entries []PersistedEntry) *PersistedEntry {
	if r == nil || r.persisted == nil {
		return nil
	}

	canonicalName := OidcConfigMapName(r.persisted.storePrefix, requestKey)
	var best *PersistedEntry
	for i := range entries {
		candidate := &entries[i]
		if betterHydrationEntry(candidate, best, canonicalName) {
			best = candidate
		}
	}
	return best
}

func betterHydrationEntry(candidate, current *PersistedEntry, canonicalName string) bool {
	if candidate == nil || candidate.Provider == nil {
		return false
	}
	if current == nil || current.Provider == nil {
		return true
	}

	switch {
	case candidate.Provider.FetchedAt.After(current.Provider.FetchedAt):
		return true
	case current.Provider.FetchedAt.After(candidate.Provider.FetchedAt):
		return false
	}

	candidateCanonical := candidate.NamespacedName.Name == canonicalName
	currentCanonical := current.NamespacedName.Name == canonicalName
	if candidateCanonical != currentCanonical {
		return candidateCanonical
	}

	if candidate.NamespacedName.Name != current.NamespacedName.Name {
		return candidate.NamespacedName.Name < current.NamespacedName.Name
	}
	return candidate.NamespacedName.Namespace < current.NamespacedName.Namespace
}

func providersEqual(a, b *DiscoveredProvider) bool {
	switch {
	case a == nil && b == nil:
		return true
	case a == nil || b == nil:
		return false
	default:
		return reflect.DeepEqual(*a, *b)
	}
}

func normalizePersistedProvider(storePrefix, configMapName string, provider DiscoveredProvider) DiscoveredProvider {
	if provider.IssuerURL == "" {
		return provider
	}

	// Re-derive the request key from the issuer discovery URL and verify the
	// ConfigMap name matches to detect and fix stale/migrated request keys.
	requestKeyFromURL, err := requestKeyForDirectIssuer(provider.IssuerURL)
	if err != nil {
		return provider
	}
	if OidcConfigMapName(storePrefix, requestKeyFromURL) == configMapName {
		provider.RequestKey = requestKeyFromURL
	}

	return provider
}
