package remotecache

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"

	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/metrics"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

const ComponentLabel = "app.kubernetes.io/component"

// Codec serializes a payload T to/from a ConfigMap.
type Codec[T Result[T]] struct {
	// ObservabilityName prefixes KRT collection names; keep stable across releases to avoid metric churn.
	ObservabilityName string
	Parse             func(*corev1.ConfigMap) (T, error)
	Serialize         func(*corev1.ConfigMap, T) error
}

// Entry is a parsed ConfigMap; on parse failure ParseError is set and Payload is nil.
type Entry[T Result[T]] struct {
	NamespacedName types.NamespacedName
	Payload        *T
	ParseError     string
}

// ResourceName satisfies krt.ResourceNamer.
func (e Entry[T]) ResourceName() string {
	return e.NamespacedName.String()
}

// Equals satisfies krt.Equaler by delegating to the payload's typed Equals.
func (e Entry[T]) Equals(other Entry[T]) bool {
	return e.NamespacedName == other.NamespacedName &&
		e.ParseError == other.ParseError &&
		payloadEquals(e.Payload, other.Payload)
}

func (e Entry[T]) RequestKey() (remotehttp.FetchKey, bool) {
	if e.Payload == nil {
		return "", false
	}
	return (*e.Payload).RemoteRequestKey(), true
}

func (e Entry[T]) GetName() string {
	return e.NamespacedName.Name
}

// BestHydrationEntry returns the freshest payload, breaking timestamp ties
// via canonicalName. Duplicates per key are transient migration/orphan-sweep
// artifacts where a non-canonical entry can temporarily hold newer material.
func BestHydrationEntry[T Result[T]](entries []Entry[T], canonicalName string) Entry[T] {
	var best Entry[T]
	hasBest := false
	for _, e := range entries {
		if !hasBest || isBetterHydrationEntry(e, best, canonicalName) {
			best = e
			hasBest = true
		}
	}
	return best
}

func isBetterHydrationEntry[T Result[T]](candidate, current Entry[T], canonicalName string) bool {
	switch {
	case candidate.Payload == nil && current.Payload != nil:
		return false
	case candidate.Payload != nil && current.Payload == nil:
		return true
	case candidate.Payload != nil && current.Payload != nil:
		candidateFetchedAt := (*candidate.Payload).RemoteFetchedAt()
		currentFetchedAt := (*current.Payload).RemoteFetchedAt()
		if candidateFetchedAt.After(currentFetchedAt) {
			return true
		}
		if currentFetchedAt.After(candidateFetchedAt) {
			return false
		}
	}

	candidateCanonical := candidate.NamespacedName.Name == canonicalName
	currentCanonical := current.NamespacedName.Name == canonicalName
	return candidateCanonical && !currentCanonical
}

// Entries watches Codec-labeled ConfigMaps and exposes typed lookups by request key.
type Entries[T Result[T]] struct {
	codec               Codec[T]
	storePrefix         string
	deploymentNamespace string
	entries             krt.Collection[Entry[T]]
	byRequestKey        krt.Index[remotehttp.FetchKey, Entry[T]]
	logger              *slog.Logger
}

// New filters ConfigMaps by storePrefix label in deploymentNamespace.
func New[T Result[T]](
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

	return NewFromCollection(
		codec,
		configMaps,
		storePrefix,
		deploymentNamespace,
		krtOptions.ToOptions(codec.ObservabilityName+"/Entries")...,
	)
}

// NewFromCollection lets tests supply a static ConfigMap collection.
func NewFromCollection[T Result[T]](
	codec Codec[T],
	configMaps krt.Collection[*corev1.ConfigMap],
	storePrefix, deploymentNamespace string,
	opts ...krt.CollectionOption,
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
		return &Entry[T]{
			NamespacedName: namespacedName,
			Payload:        &payload,
		}
	}, opts...)

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
		logger:              logging.New("remotecache/" + codec.ObservabilityName),
	}
}

// CanonicalGet returns only the canonical entry; legacy duplicates are ignored.
func (e *Entries[T]) CanonicalGet(krtctx krt.HandlerContext, requestKey remotehttp.FetchKey) (T, bool) {
	var zero T
	if e == nil {
		return zero, false
	}
	matches := krt.Fetch(krtctx, e.entries, krt.FilterIndex(e.byRequestKey, requestKey))
	canonicalName := e.ConfigMapName(requestKey)
	found := slices.FindFunc(matches, func(entry Entry[T]) bool {
		return entry.Payload != nil && entry.NamespacedName.Name == canonicalName
	})
	if found == nil {
		return zero, false
	}
	return *found.Payload, true
}

// LoadAll returns one payload per request key (best-of). Parse errors are joined so callers
// can alert on persistent corruption without blocking hydration of the entries that did parse.
func (e *Entries[T]) LoadAll(ctx context.Context) ([]T, error) {
	if e == nil {
		return nil, nil
	}

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
			e.logger.ErrorContext(ctx, "error deserializing ConfigMap", "error", err, "ConfigMap", entry.NamespacedName.String())
			hydrationParseErrors.Inc(metrics.Label{Name: codecLabel, Value: e.codec.ObservabilityName})
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

func (e *Entries[T]) ConfigMapName(requestKey remotehttp.FetchKey) string {
	return ConfigMapName(e.storePrefix, requestKey)
}

func (e *Entries[T]) LabelSelector() string {
	return LabelSelector(e.storePrefix)
}

func (e *Entries[T]) ConfigMapLabels() map[string]string {
	return ConfigMapLabels(e.storePrefix)
}

func (e *Entries[T]) Collection() krt.Collection[Entry[T]] {
	return e.entries
}

// EntriesForRequestKey returns all entries (canonical + legacy/duplicate) indexed under requestKey.
func (e *Entries[T]) EntriesForRequestKey(requestKey remotehttp.FetchKey) []Entry[T] {
	if e == nil {
		return nil
	}
	return e.byRequestKey.Lookup(requestKey)
}

// Serialize writes record into cm via the codec.
func (e *Entries[T]) Serialize(cm *corev1.ConfigMap, record T) error {
	return e.codec.Serialize(cm, record)
}

// ConfigMapName returns the canonical ConfigMap name for a (storePrefix,
// requestKey) pair. Format: <storePrefix>-<sha256(requestKey)>.
func ConfigMapName(storePrefix string, requestKey remotehttp.FetchKey) string {
	sum := sha256.Sum256([]byte(requestKey))
	return fmt.Sprintf("%s-%s", storePrefix, hex.EncodeToString(sum[:]))
}

func LabelSelector(storePrefix string) string {
	return ComponentLabel + "=" + storePrefix
}

func ConfigMapLabels(storePrefix string) map[string]string {
	return map[string]string{ComponentLabel: storePrefix}
}

func payloadEquals[T Result[T]](a, b *T) bool {
	switch {
	case a == nil && b == nil:
		return true
	case a == nil || b == nil:
		return false
	default:
		return (*a).Equals(*b)
	}
}
