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
		ObservabilityName: observabilityName,
		Parse:             JwksFromConfigMap,
		Serialize:         SetJwksInConfigMap,
	}
}

// NewPersistedEntries constructs the KRT-backed JWKS persistence layer.
func NewPersistedEntries(client apiclient.Client, krtOptions krtutil.KrtOptions, deploymentNamespace string) *PersistedEntries {
	return remotecache.New(JwksCodec(), client, krtOptions, DefaultJwksStorePrefix, deploymentNamespace)
}

// NewPersistedEntriesFromCollection constructs a PersistedEntries from an
// existing ConfigMap collection. Useful for testing with static collections.
func NewPersistedEntriesFromCollection(configMaps krt.Collection[*corev1.ConfigMap], deploymentNamespace string) *PersistedEntries {
	return remotecache.NewFromCollection(JwksCodec(), configMaps, DefaultJwksStorePrefix, deploymentNamespace)
}

// JwksFromConfigMap parses a Keyset from a ConfigMap, falling back to the
// pre-#1175 single-entry map format so old persisted state still hydrates.
func JwksFromConfigMap(cm *corev1.ConfigMap) (Keyset, error) {
	jwksStore := cm.Data[configMapKey]

	var keyset Keyset
	currentErr := json.Unmarshal([]byte(jwksStore), &keyset)
	if currentErr == nil && keyset.RequestKey != "" {
		return keyset, nil
	}

	// Fallback to legacy map format
	var legacy map[string]string
	if legacyErr := json.Unmarshal([]byte(jwksStore), &legacy); legacyErr != nil {
		return Keyset{}, fmt.Errorf("failed to unmarshal current and legacy formats: %w", errors.Join(currentErr, legacyErr))
	}
	if len(legacy) != 1 {
		return Keyset{}, fmt.Errorf("unexpected legacy jwks payload: expected 1 entry, got %d", len(legacy))
	}

	for uri, jwksJSON := range legacy {
		// Zero FetchedAt forces refresh-and-rewrite on first reconcile to migrate legacy entries.
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
