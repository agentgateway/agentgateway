package jwks

import (
	"encoding/json"
	"errors"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// configMapKey is the key inside a JWKS-store ConfigMap's Data map that holds
// the serialized Keyset.
const configMapKey = "jwks-store"

// observabilityName names the JWKS persisted-cache subsystem in KRT collection
// metric labels. Stable across releases for metric continuity.
const observabilityName = "persisted_jwks"

// PersistedEntry is the parsed persisted JWKS record view for a single
// ConfigMap.
type PersistedEntry = remotecache.Entry[Keyset]

// PersistedEntries is the JWKS-specific KRT view over JWKS-store ConfigMaps.
type PersistedEntries = remotecache.Entries[Keyset]

// JwksCodec returns the persistedcache codec for JWKS keysets, including the
// legacy single-entry-map fallback that JwksFromConfigMap implements.
func JwksCodec() remotecache.Codec[Keyset] {
	return remotecache.Codec[Keyset]{
		DataKey:           configMapKey,
		ObservabilityName: observabilityName,
		Parse:             JwksFromConfigMap,
		Serialize:         SetJwksInConfigMap,
		Normalize:         normalizePersistedKeyset,
	}
}

// NewPersistedEntries constructs the KRT-backed JWKS persistence layer.
func NewPersistedEntries(client apiclient.Client, krtOptions krtutil.KrtOptions, storePrefix, deploymentNamespace string) *PersistedEntries {
	return remotecache.New(JwksCodec(), client, krtOptions, storePrefix, deploymentNamespace)
}

// NewPersistedEntriesFromCollection constructs a PersistedEntries from an
// existing ConfigMap collection. Useful for testing with static collections.
func NewPersistedEntriesFromCollection(configMaps krt.Collection[*corev1.ConfigMap], storePrefix, deploymentNamespace string) *PersistedEntries {
	return remotecache.NewFromCollection(JwksCodec(), configMaps, storePrefix, deploymentNamespace)
}

// JwksFromConfigMap parses a Keyset from a ConfigMap, falling back to the
// pre-#1618 single-entry map format when present so old persisted state still
// hydrates after upgrade.
func JwksFromConfigMap(cm *corev1.ConfigMap) (Keyset, error) {
	jwksStore := cm.Data[configMapKey]

	var keyset Keyset
	if err := json.Unmarshal([]byte(jwksStore), &keyset); err == nil && keyset.RequestKey != "" {
		return keyset, nil
	}

	// Fallback to legacy map format
	var legacy map[string]string
	if err := json.Unmarshal([]byte(jwksStore), &legacy); err != nil {
		return Keyset{}, fmt.Errorf("failed to unmarshal current and legacy formats: %w", err)
	}
	if len(legacy) != 1 {
		return Keyset{}, fmt.Errorf("unexpected legacy jwks payload: expected 1 entry, got %d", len(legacy))
	}

	for uri, jwksJSON := range legacy {
		return Keyset{
			RequestKey: remotehttp.FetchTarget{URL: uri}.Key(),
			URL:        uri,
			JwksJSON:   jwksJSON,
		}, nil
	}

	// unreachable after len==1 check, but satisfies the compiler
	return Keyset{}, errors.New("unexpected legacy jwks state")
}

// SetJwksInConfigMap serializes a Keyset into a ConfigMap's Data map.
func SetJwksInConfigMap(cm *corev1.ConfigMap, keyset Keyset) error {
	b, err := json.Marshal(keyset)
	if err != nil {
		return err
	}
	if cm.Data == nil {
		cm.Data = make(map[string]string)
	}
	cm.Data[configMapKey] = string(b)
	return nil
}

// normalizePersistedKeyset repairs a parsed Keyset's RequestKey when its
// derivation logic has evolved. If the ConfigMap's name matches the canonical
// hash for the URL-derived request key, rewrite RequestKey to the current
// derivation so lookups by it succeed after upgrade.
func normalizePersistedKeyset(storePrefix, configMapName string, keyset Keyset) Keyset {
	if keyset.URL == "" {
		return keyset
	}

	requestKeyFromURL := remotehttp.FetchTarget{URL: keyset.URL}.Key()
	if remotecache.ConfigMapName(storePrefix, requestKeyFromURL) == configMapName {
		keyset.RequestKey = requestKeyFromURL
	}

	return keyset
}
