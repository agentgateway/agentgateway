package remotecache

import (
	"context"
	"log/slog"
	"maps"
	"math"
	"time"

	"golang.org/x/time/rate"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	"k8s.io/client-go/util/workqueue"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
)

// rateLimiter: per-item exponential backoff under a 10qps/100-burst token bucket.
var rateLimiter = workqueue.NewTypedMaxOfRateLimiter(
	workqueue.NewTypedItemExponentialFailureRateLimiter[any](500*time.Millisecond, 10*time.Second),
	&workqueue.TypedBucketRateLimiter[any]{Limiter: rate.NewLimiter(rate.Limit(10), 100)},
)

// ConfigMapController syncs the fetched-results KRT collection into ConfigMaps.
type ConfigMapController[T Result[T]] struct {
	apiClient           apiclient.Client
	cmClient            kclient.Client[*corev1.ConfigMap]
	eventQueue          controllers.Queue
	results             krt.Collection[T]
	entries             *Entries[T]
	deploymentNamespace string
	controllerName      string
	waitForSync         []cache.InformerSynced
	logger              *slog.Logger
}

// Init builds the informer + workqueue. Must be called at setup time, BEFORE
// the parent kube.Client's informers start — kclient.NewFiltered registers a
// new informer that joins the shared factory, and late-added informers never
// sync.
func (c *ConfigMapController[T]) Init() {
	c.cmClient = kclient.NewFiltered[*corev1.ConfigMap](c.apiClient,
		kclient.Filter{
			ObjectFilter:  c.apiClient.ObjectFilter(),
			Namespace:     c.deploymentNamespace,
			LabelSelector: c.entries.LabelSelector(),
		})

	c.waitForSync = append([]cache.InformerSynced{c.cmClient.HasSynced}, c.waitForSync...)

	c.eventQueue = controllers.NewQueue(
		c.controllerName,
		controllers.WithReconciler(c.Reconcile),
		controllers.WithMaxAttempts(math.MaxInt),
		controllers.WithRateLimiter(rateLimiter),
	)
}

func (c *ConfigMapController[T]) Start(ctx context.Context) error {
	c.logger.Info("waiting for cache to sync")
	c.apiClient.Core().WaitForCacheSync(
		c.controllerName,
		ctx.Done(),
		c.waitForSync...,
	)

	c.logger.Info("starting ConfigMap controller")
	resultRegistration := c.results.RegisterBatch(func(events []krt.Event[T]) {
		for _, event := range events {
			c.enqueueFetchedResult(event.Old)
			c.enqueueFetchedResult(event.New)
		}
	}, true)
	defer resultRegistration.UnregisterHandler()

	persistedRegistration := c.entries.Collection().RegisterBatch(func(events []krt.Event[Entry[T]]) {
		for _, event := range events {
			c.enqueuePersistedEntry(event.Old)
			c.enqueuePersistedEntry(event.New)
		}
	}, true)
	defer persistedRegistration.UnregisterHandler()

	go c.eventQueue.Run(ctx.Done())

	if !resultRegistration.WaitUntilSynced(ctx.Done()) {
		return nil
	}
	if !persistedRegistration.WaitUntilSynced(ctx.Done()) {
		return nil
	}

	<-ctx.Done()
	return nil
}

func (c *ConfigMapController[T]) Reconcile(req types.NamespacedName) error {
	c.logger.Debug("syncing fetched result to ConfigMap(s)")
	requestKey := remotehttp.FetchKey(req.Name)

	existingEntries := c.entries.EntriesForRequestKey(requestKey)
	existingNames := slices.Map(existingEntries, func(entry Entry[T]) string {
		return entry.GetName()
	})

	result := c.results.GetKey(string(requestKey))
	var record T
	exists := result != nil
	var canonicalName string
	if exists {
		record = *result
		canonicalName = c.entries.ConfigMapName(requestKey)
	}

	plan := PlanConfigMapSync(existingNames, canonicalName, exists)

	if plan.UpsertName != "" {
		if err := c.upsertConfigMap(req.Namespace, plan.UpsertName, record); err != nil {
			return err
		}
	}
	for _, deleteName := range plan.DeleteNames {
		c.logger.Debug("deleting ConfigMap", "name", deleteName)
		if err := client.IgnoreNotFound(c.cmClient.Delete(deleteName, req.Namespace)); err != nil {
			return err
		}
	}

	return nil
}

func (c *ConfigMapController[T]) NeedLeaderElection() bool {
	return true
}

// RunnableName satisfies common.NamedRunnable so setup's dedup check catches
// duplicate per-codec controller registrations.
func (c *ConfigMapController[T]) RunnableName() string {
	return c.controllerName
}

func (c *ConfigMapController[T]) newStoreConfigMap(name string) *corev1.ConfigMap {
	return &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: c.deploymentNamespace,
			Labels:    c.entries.ConfigMapLabels(),
		},
		Data: make(map[string]string),
	}
}

func (c *ConfigMapController[T]) enqueueFetchedResult(record *T) {
	if record == nil {
		return
	}
	c.eventQueue.Add(RequestQueueKey(c.deploymentNamespace, (*record).RemoteRequestKey()))
}

func (c *ConfigMapController[T]) enqueuePersistedEntry(entry *Entry[T]) {
	if entry == nil {
		return
	}
	requestKey, ok := entry.RequestKey()
	if !ok {
		return
	}
	c.eventQueue.Add(RequestQueueKey(c.deploymentNamespace, requestKey))
}

func (c *ConfigMapController[T]) upsertConfigMap(namespace, name string, record T) error {
	existingCmRaw := c.cmClient.Get(name, namespace)
	if existingCmRaw == nil {
		c.logger.Debug("creating ConfigMap", "name", name)
		newCm := c.newStoreConfigMap(name)
		if err := c.entries.Serialize(newCm, record); err != nil {
			c.logger.Error("error setting record in ConfigMap", "error", err)
			return err
		}

		if _, err := c.cmClient.Create(newCm); err != nil {
			c.logger.Error("error creating ConfigMap", "error", err)
			return err
		}
		return nil
	}

	existingCm := existingCmRaw.DeepCopy()
	if err := c.entries.Serialize(existingCm, record); err != nil {
		c.logger.Error("error setting record in ConfigMap", "error", err)
		return err
	}
	// Skip no-op writes: each Update echoes back through the informer and
	// re-enqueues this key, so an unconditional write doubles every PUT and
	// replays one PUT per cached entry on startup right after hydration
	// loaded identical data.
	if maps.Equal(existingCmRaw.Data, existingCm.Data) {
		return nil
	}
	c.logger.Debug("updating ConfigMap", "name", name)
	if _, err := c.cmClient.Update(existingCm); err != nil {
		c.logger.Error("error updating ConfigMap", "error", err)
		return err
	}
	return nil
}

type SyncPlan struct {
	UpsertName  string
	DeleteNames []string
}

// PlanConfigMapSync chooses upsert/delete names from existing names and the canonical name.
func PlanConfigMapSync(existingNames []string, canonicalName string, exists bool) SyncPlan {
	if exists {
		return SyncPlan{
			UpsertName: canonicalName,
			DeleteNames: slices.Filter(existingNames, func(name string) bool {
				return name != canonicalName
			}),
		}
	}

	return SyncPlan{
		DeleteNames: existingNames,
	}
}

func RequestQueueKey(namespace string, requestKey remotehttp.FetchKey) types.NamespacedName {
	return types.NamespacedName{
		Namespace: namespace,
		Name:      string(requestKey),
	}
}

// ResultStore is the subset of a subsystem Store that
// NewStoreConfigMapController consumes: the fetched-results collection plus a
// readiness signal.
type ResultStore[R Result[R]] interface {
	FetchedResults() *FetchedResults[R]
	HasSynced() bool
}

// NewStoreConfigMapController wires a subsystem Store and its persisted Entries
// into a ConfigMapController. It replaces the identical per-subsystem
// constructor boilerplate; callers supply only the controller name, store,
// entries and logger.
func NewStoreConfigMapController[R Result[R]](
	apiClient apiclient.Client,
	deploymentNamespace string,
	controllerName string,
	store ResultStore[R],
	entries *Entries[R],
	logger *slog.Logger,
) *ConfigMapController[R] {
	logger.Info("creating store configmap controller", "controller", controllerName)
	results := store.FetchedResults().Collection()
	return &ConfigMapController[R]{
		apiClient:           apiClient,
		deploymentNamespace: deploymentNamespace,
		controllerName:      controllerName,
		results:             results,
		entries:             entries,
		waitForSync:         []cache.InformerSynced{results.HasSynced, entries.Collection().HasSynced, store.HasSynced},
		logger:              logger,
	}
}
