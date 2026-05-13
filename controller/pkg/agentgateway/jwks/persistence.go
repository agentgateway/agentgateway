package jwks

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"

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

const configMapKey = "jwks-store"
const jwksStoreComponentLabel = "app.kubernetes.io/component"

func JwksStoreLabelSelector(storePrefix string) string {
	return jwksStoreComponentLabel + "=" + storePrefix
}

func JwksStoreConfigMapLabel(storePrefix string) map[string]string {
	return map[string]string{jwksStoreComponentLabel: storePrefix}
}

// PersistedEntry is the parsed persisted JWKS artifact view for a single
// ConfigMap. It preserves the backing ConfigMap identity so callers can reason
// about canonical and legacy artifacts for the same request key.
type PersistedEntry struct {
	NamespacedName types.NamespacedName
	Keyset         *Keyset
	ParseError     string
}

func (e PersistedEntry) ResourceName() string {
	return e.NamespacedName.String()
}

func (e PersistedEntry) Equals(other PersistedEntry) bool {
	return e.NamespacedName == other.NamespacedName &&
		e.ParseError == other.ParseError &&
		keysetsEqual(e.Keyset, other.Keyset)
}

func (e PersistedEntry) RequestKey() (remotehttp.FetchKey, bool) {
	if e.Keyset == nil {
		return "", false
	}
	return e.Keyset.RequestKey, true
}

type PersistedEntries struct {
	storePrefix  string
	entries      krt.Collection[PersistedEntry]
	byRequestKey krt.Index[remotehttp.FetchKey, PersistedEntry]
}

// keysetCache provides canonical lookup semantics over the shared persisted JWKS
// collection. Inline JWKS resolution only trusts the canonical ConfigMap name.
type keysetCache struct {
	persisted *PersistedEntries
}

// persistedKeysetReader provides hydration semantics over the shared persisted
// JWKS collection. Startup loading may fall back to legacy/non-canonical
// artifacts while migration cleanup converges persisted state.
type persistedKeysetReader struct {
	persisted *PersistedEntries
}

func NewPersistedEntries(client apiclient.Client, krtOptions krtutil.KrtOptions, storePrefix, deploymentNamespace string) *PersistedEntries {
	configMaps := krt.NewFilteredInformer[*corev1.ConfigMap](client, kclient.Filter{
		ObjectFilter:  client.ObjectFilter(),
		Namespace:     deploymentNamespace,
		LabelSelector: JwksStoreLabelSelector(storePrefix),
	}, krtOptions.ToOptions("persisted_jwks/ConfigMaps")...)

	return NewPersistedEntriesFromCollection(configMaps, storePrefix, deploymentNamespace)
}

func NewPersistedEntriesFromCollection(configMaps krt.Collection[*corev1.ConfigMap], storePrefix, deploymentNamespace string) *PersistedEntries {
	entries := krt.NewCollection(configMaps, func(krtctx krt.HandlerContext, cm *corev1.ConfigMap) *PersistedEntry {
		if cm == nil {
			return nil
		}
		if cm.Namespace != deploymentNamespace {
			return nil
		}
		if cm.Labels[jwksStoreComponentLabel] != storePrefix {
			return nil
		}

		entry := PersistedEntry{
			NamespacedName: types.NamespacedName{
				Namespace: cm.Namespace,
				Name:      cm.Name,
			},
		}
		keyset, err := JwksFromConfigMap(cm)
		if err != nil {
			entry.ParseError = err.Error()
			return &entry
		}
		entry.Keyset = &keyset
		return &entry
	})

	return &PersistedEntries{
		storePrefix: storePrefix,
		entries:     entries,
		byRequestKey: krt.NewIndex(entries, "persisted-jwks-request-key", func(entry PersistedEntry) []remotehttp.FetchKey {
			requestKey, ok := entry.RequestKey()
			if !ok {
				return nil
			}
			return []remotehttp.FetchKey{requestKey}
		}),
	}
}

func newKeysetCache(persisted *PersistedEntries) *keysetCache {
	if persisted == nil {
		return nil
	}
	return &keysetCache{persisted: persisted}
}

func newPersistedKeysetReader(persisted *PersistedEntries) *persistedKeysetReader {
	if persisted == nil {
		return nil
	}
	return &persistedKeysetReader{persisted: persisted}
}

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

func RequestKeyFromConfigMap(cm *corev1.ConfigMap) (remotehttp.FetchKey, error) {
	keyset, err := JwksFromConfigMap(cm)
	if err != nil {
		return "", err
	}
	return keyset.RequestKey, nil
}

func JwksConfigMapName(storePrefix string, requestKey remotehttp.FetchKey) string {
	sum := sha256.Sum256([]byte(requestKey))
	return fmt.Sprintf("%s-%s", storePrefix, hex.EncodeToString(sum[:]))
}

func JwksConfigMapNamespacedName(storePrefix, namespace string, requestKey remotehttp.FetchKey) types.NamespacedName {
	return types.NamespacedName{
		Namespace: namespace,
		Name:      JwksConfigMapName(storePrefix, requestKey),
	}
}

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

func (ps *PersistedEntries) entriesForRequestKey(requestKey remotehttp.FetchKey) []PersistedEntry {
	return ps.byRequestKey.Lookup(requestKey)
}

func (c *keysetCache) Get(krtctx krt.HandlerContext, requestKey remotehttp.FetchKey) (Keyset, bool) {
	if c == nil || c.persisted == nil {
		return Keyset{}, false
	}

	entries := krt.Fetch(krtctx, c.persisted.entries, krt.FilterIndex(c.persisted.byRequestKey, requestKey))
	canonicalName := JwksConfigMapName(c.persisted.storePrefix, requestKey)
	for _, entry := range entries {
		if entry.Keyset == nil {
			continue
		}
		if entry.NamespacedName.Name == canonicalName {
			return *entry.Keyset, true
		}
	}
	return Keyset{}, false
}

func (r *persistedKeysetReader) LoadPersistedKeysets(ctx context.Context) ([]Keyset, error) {
	if r == nil || r.persisted == nil {
		return nil, nil
	}

	log := log.FromContext(ctx)

	kube.WaitForCacheSync("JWKS persisted keysets", ctx.Done(), r.persisted.entries.HasSynced)

	allPersistedEntries := r.persisted.entries.List()
	if len(allPersistedEntries) == 0 {
		return nil, nil
	}

	errs := make([]error, 0)
	entriesByRequestKey := make(map[remotehttp.FetchKey][]PersistedEntry)
	for _, entry := range allPersistedEntries {
		requestKey, ok := entry.RequestKey()
		if !ok {
			err := fmt.Errorf("error deserializing jwks ConfigMap %s: %s", entry.NamespacedName.String(), entry.ParseError)
			log.Error(err, "error deserializing jwks ConfigMap", "ConfigMap", entry.NamespacedName.String())
			errs = append(errs, err)
			continue
		}
		entriesByRequestKey[requestKey] = append(entriesByRequestKey[requestKey], entry)
	}

	keysets := make([]Keyset, 0, len(entriesByRequestKey))
	for requestKey, entries := range entriesByRequestKey {
		keyset, ok := r.hydrationKeyset(requestKey, entries)
		if !ok {
			continue
		}
		keysets = append(keysets, keyset)
	}

	return keysets, errors.Join(errs...)
}

func (r *persistedKeysetReader) hydrationKeyset(requestKey remotehttp.FetchKey, entries []PersistedEntry) (Keyset, bool) {
	best := r.bestHydrationEntry(requestKey, entries)
	if best == nil || best.Keyset == nil {
		return Keyset{}, false
	}
	return *best.Keyset, true
}

func (r *persistedKeysetReader) bestHydrationEntry(requestKey remotehttp.FetchKey, entries []PersistedEntry) *PersistedEntry {
	if r == nil || r.persisted == nil {
		return nil
	}

	canonicalName := JwksConfigMapName(r.persisted.storePrefix, requestKey)
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
	if candidate == nil || candidate.Keyset == nil {
		return false
	}
	if current == nil || current.Keyset == nil {
		return true
	}

	switch {
	case candidate.Keyset.FetchedAt.After(current.Keyset.FetchedAt):
		return true
	case current.Keyset.FetchedAt.After(candidate.Keyset.FetchedAt):
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

func keysetsEqual(a, b *Keyset) bool {
	switch {
	case a == nil && b == nil:
		return true
	case a == nil || b == nil:
		return false
	default:
		return *a == *b
	}
}
