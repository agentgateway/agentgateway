package remotecache

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"reflect"
	"time"

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

// ComponentLabel is the Kubernetes well-known label key used to identify
// ConfigMaps managed by a remotecache subsystem.
const ComponentLabel = "app.kubernetes.io/component"

// Codec adapts a typed payload T to the generic ConfigMap-backed persistence
// layer. Subsystems supply one Codec instance and consume Entries[T].
//
// T is constrained to Result so the generic Entry can extract
// request keys and hydration timestamps without per-subsystem callbacks.
type Codec[T Result] struct {
	// DataKey is the key inside ConfigMap.Data that holds the serialized T.
	DataKey string
	// ObservabilityName is the prefix used for KRT collection names; affects
	// metric labels. Stable strings preserve continuity across releases.
	ObservabilityName string
	// Parse deserializes a ConfigMap into T. Subsystem-specific (legacy
	// fallbacks, multi-format support) live inside this callback.
	Parse func(*corev1.ConfigMap) (T, error)
	// Serialize writes T into a ConfigMap's Data map under DataKey.
	Serialize func(*corev1.ConfigMap, T) error
	// Normalize repairs a parsed payload's identity fields (e.g. RequestKey)
	// when its derivation logic has evolved across releases.
	Normalize func(storePrefix, configMapName string, payload T) T
}

// Entry is the parsed view of a single ConfigMap. ParseError is set when Parse
// fails; Payload is nil in that case.
type Entry[T Result] struct {
	NamespacedName types.NamespacedName
	Payload        *T
	ParseError     string
}

// ResourceName satisfies krt.ResourceNamer.
func (e Entry[T]) ResourceName() string {
	return e.NamespacedName.String()
}

// Equals satisfies krt's equality requirement. Uses reflect.DeepEqual so
// payloads with slice fields compare correctly without per-subsystem callbacks.
func (e Entry[T]) Equals(other Entry[T]) bool {
	return e.NamespacedName == other.NamespacedName &&
		e.ParseError == other.ParseError &&
		payloadEquals(e.Payload, other.Payload)
}

// RequestKey satisfies PersistedRecord. Returns ok=false when Payload is nil.
func (e Entry[T]) RequestKey() (remotehttp.FetchKey, bool) {
	if e.Payload == nil {
		return "", false
	}
	return (*e.Payload).RemoteRequestKey(), true
}

// GetName satisfies PersistedRecord and Hydratable.
func (e Entry[T]) GetName() string {
	return e.NamespacedName.Name
}

// GetNamespace satisfies Hydratable.
func (e Entry[T]) GetNamespace() string {
	return e.NamespacedName.Namespace
}

// GetFetchedAt satisfies Hydratable. Returns the zero time when Payload is
// nil so unparsable entries lose all hydration tie-breaks.
func (e Entry[T]) GetFetchedAt() time.Time {
	if e.Payload == nil {
		return time.Time{}
	}
	return (*e.Payload).RemoteFetchedAt()
}

var (
	_ PersistedRecord = Entry[Result]{}
	_ Hydratable      = Entry[Result]{}
)

// Hydratable provides the metadata needed to pick the "best" persisted entry
// for a given request key during startup hydration.
type Hydratable interface {
	GetFetchedAt() time.Time
	GetName() string
	GetNamespace() string
}

// BestHydrationEntry selects the best entry from a list of persisted entries
// for the same request key. It prefers the latest FetchedAt, then the
// canonical name, then lexicographical order.
func BestHydrationEntry[T Hydratable](entries []T, canonicalName string) T {
	var best T
	var found bool
	for _, candidate := range entries {
		if !found || isBetterHydrationEntry(candidate, best, canonicalName) {
			best = candidate
			found = true
		}
	}
	return best
}

// isBetterHydrationEntry returns true if candidate is preferred over current.
func isBetterHydrationEntry[T Hydratable](candidate, current T, canonicalName string) bool {
	candidateFetchedAt := candidate.GetFetchedAt()
	currentFetchedAt := current.GetFetchedAt()

	switch {
	case candidateFetchedAt.After(currentFetchedAt):
		return true
	case currentFetchedAt.After(candidateFetchedAt):
		return false
	}

	candidateCanonical := candidate.GetName() == canonicalName
	currentCanonical := current.GetName() == canonicalName
	if candidateCanonical != currentCanonical {
		return candidateCanonical
	}

	if candidate.GetName() != current.GetName() {
		return candidate.GetName() < current.GetName()
	}
	return candidate.GetNamespace() < current.GetNamespace()
}

// Entries is the generic read-side persistence layer. It watches ConfigMaps
// labeled for a single subsystem, parses each via the supplied Codec, and
// exposes typed lookups by request key plus startup hydration.
type Entries[T Result] struct {
	codec               Codec[T]
	storePrefix         string
	deploymentNamespace string
	entries             krt.Collection[Entry[T]]
	byRequestKey        krt.Index[remotehttp.FetchKey, Entry[T]]
}

// New constructs an Entries by spinning up a filtered ConfigMap informer in
// deploymentNamespace selecting on the codec's component label set to
// storePrefix.
func New[T Result](
	codec Codec[T],
	client apiclient.Client,
	krtOptions krtutil.KrtOptions,
	storePrefix, deploymentNamespace string,
) *Entries[T] {
	configMaps := krt.NewFilteredInformer[*corev1.ConfigMap](client, kclient.Filter{
		ObjectFilter:  client.ObjectFilter(),
		Namespace:     deploymentNamespace,
		LabelSelector: LabelSelector(storePrefix),
	}, krtOptions.ToOptions(codec.ObservabilityName+"/ConfigMaps")...)

	return NewFromCollection(codec, configMaps, storePrefix, deploymentNamespace)
}

// NewFromCollection constructs an Entries on top of an existing ConfigMap
// collection. Useful for tests that supply a static collection.
func NewFromCollection[T Result](
	codec Codec[T],
	configMaps krt.Collection[*corev1.ConfigMap],
	storePrefix, deploymentNamespace string,
) *Entries[T] {
	entries := krt.NewCollection(configMaps, func(_ krt.HandlerContext, cm *corev1.ConfigMap) *Entry[T] {
		if cm == nil {
			return nil
		}
		if cm.Namespace != deploymentNamespace {
			return nil
		}
		if cm.Labels[ComponentLabel] != storePrefix {
			return nil
		}

		namespacedName := types.NamespacedName{
			Namespace: cm.Namespace,
			Name:      cm.Name,
		}
		payload, err := codec.Parse(cm)
		if err != nil {
			return &Entry[T]{
				NamespacedName: namespacedName,
				ParseError:     err.Error(),
			}
		}
		payload = codec.Normalize(storePrefix, cm.Name, payload)
		return &Entry[T]{
			NamespacedName: namespacedName,
			Payload:        &payload,
		}
	})

	byRequestKey := krt.NewIndex(entries, codec.ObservabilityName+"-request-key", func(entry Entry[T]) []remotehttp.FetchKey {
		requestKey, ok := entry.RequestKey()
		if !ok {
			return nil
		}
		return []remotehttp.FetchKey{requestKey}
	})

	return &Entries[T]{
		codec:               codec,
		storePrefix:         storePrefix,
		deploymentNamespace: deploymentNamespace,
		entries:             entries,
		byRequestKey:        byRequestKey,
	}
}

// CanonicalGet returns the payload from the canonical ConfigMap (named
// ConfigMapName(requestKey)) for the given request key, if present.
// Non-canonical entries for the same key are ignored: only the canonical name
// is trusted as the authoritative cached value.
func (e *Entries[T]) CanonicalGet(krtctx krt.HandlerContext, requestKey remotehttp.FetchKey) (T, bool) {
	var zero T
	if e == nil {
		return zero, false
	}
	matches := krt.Fetch(krtctx, e.entries, krt.FilterIndex(e.byRequestKey, requestKey))
	canonicalName := e.ConfigMapName(requestKey)
	for _, entry := range matches {
		if entry.Payload == nil {
			continue
		}
		if entry.NamespacedName.Name == canonicalName {
			return *entry.Payload, true
		}
	}
	return zero, false
}

// LoadAll returns one payload per request key, picking the best entry per
// BestHydrationEntry. Parse errors are surfaced via errors.Join so callers
// can log/alert on persistent corruption while still hydrating everything
// that did parse.
func (e *Entries[T]) LoadAll(ctx context.Context) ([]T, error) {
	if e == nil {
		return nil, nil
	}

	logger := log.FromContext(ctx)
	kube.WaitForCacheSync(e.codec.ObservabilityName, ctx.Done(), e.entries.HasSynced)

	all := e.entries.List()
	if len(all) == 0 {
		return nil, nil
	}

	var errs []error
	byKey := make(map[remotehttp.FetchKey][]Entry[T])
	for _, entry := range all {
		requestKey, ok := entry.RequestKey()
		if !ok {
			err := fmt.Errorf("error deserializing %s ConfigMap %s: %s", e.codec.ObservabilityName, entry.NamespacedName.String(), entry.ParseError)
			logger.Error(err, "error deserializing ConfigMap", "ConfigMap", entry.NamespacedName.String())
			errs = append(errs, err)
			continue
		}
		byKey[requestKey] = append(byKey[requestKey], entry)
	}

	out := make([]T, 0, len(byKey))
	for requestKey, entries := range byKey {
		best := BestHydrationEntry(entries, e.ConfigMapName(requestKey))
		if best.Payload == nil {
			continue
		}
		out = append(out, *best.Payload)
	}

	return out, errors.Join(errs...)
}

// ConfigMapName returns the canonical ConfigMap name for a given request key.
func (e *Entries[T]) ConfigMapName(requestKey remotehttp.FetchKey) string {
	return ConfigMapName(e.storePrefix, requestKey)
}

// LabelSelector returns the label selector for ConfigMaps managed by this
// Entries instance.
func (e *Entries[T]) LabelSelector() string {
	return LabelSelector(e.storePrefix)
}

// ConfigMapLabels returns the label map applied to managed ConfigMaps.
func (e *Entries[T]) ConfigMapLabels() map[string]string {
	return ConfigMapLabels(e.storePrefix)
}

// Collection exposes the underlying KRT collection so external machinery
// (e.g. ConfigMapController) can wire into it.
func (e *Entries[T]) Collection() krt.Collection[Entry[T]] {
	return e.entries
}

// EntriesForRequestKey returns all entries (canonical + legacy/duplicate)
// indexed under the given request key.
func (e *Entries[T]) EntriesForRequestKey(requestKey remotehttp.FetchKey) []Entry[T] {
	if e == nil {
		return nil
	}
	return e.byRequestKey.Lookup(requestKey)
}

// ConfigMapName returns the canonical ConfigMap name for a (storePrefix,
// requestKey) pair. Format: <storePrefix>-<sha256(requestKey)>.
func ConfigMapName(storePrefix string, requestKey remotehttp.FetchKey) string {
	sum := sha256.Sum256([]byte(requestKey))
	return fmt.Sprintf("%s-%s", storePrefix, hex.EncodeToString(sum[:]))
}

// LabelSelector returns the label selector string for ConfigMaps at a given
// store prefix.
func LabelSelector(storePrefix string) string {
	return ComponentLabel + "=" + storePrefix
}

// ConfigMapLabels returns the label map applied to ConfigMaps at a given
// store prefix.
func ConfigMapLabels(storePrefix string) map[string]string {
	return map[string]string{ComponentLabel: storePrefix}
}

func payloadEquals[T Result](a, b *T) bool {
	switch {
	case a == nil && b == nil:
		return true
	case a == nil || b == nil:
		return false
	default:
		return reflect.DeepEqual(*a, *b)
	}
}
