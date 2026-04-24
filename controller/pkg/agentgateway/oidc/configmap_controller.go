package oidc

import (
	"context"
	"math"
	"time"

	"golang.org/x/time/rate"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	"k8s.io/client-go/util/workqueue"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

// ConfigMapController synchronizes persisted OIDC providers to ConfigMaps.

var cmLogger = logging.New("oidc_store_config_map_controller")

// ConfigMapController reconciles the in-memory OIDC store against ConfigMap state.
type ConfigMapController struct {
	apiClient           apiclient.Client
	cmClient            kclient.Client[*corev1.ConfigMap]
	eventQueue          controllers.Queue
	oidcUpdates         <-chan sets.Set[remotehttp.FetchKey]
	persistedEntries    *PersistedEntries
	store               *Store
	deploymentNamespace string
	storePrefix         string
	reconcileCtx        context.Context
	waitForSync         []cache.InformerSynced
}

var (
	rateLimiter = workqueue.NewTypedMaxOfRateLimiter(
		workqueue.NewTypedItemExponentialFailureRateLimiter[any](500*time.Millisecond, 10*time.Second),
		// 10 qps, 100 bucket size.
		&workqueue.TypedBucketRateLimiter[any]{Limiter: rate.NewLimiter(rate.Limit(10), 100)},
	)
)

type configMapSyncPlan struct {
	upsertName  string
	provider    *DiscoveredProvider
	deleteNames []string
}

// NewConfigMapController constructs a ConfigMapController.
func NewConfigMapController(apiClient apiclient.Client, storePrefix, deploymentNamespace string, store *Store, persistedEntries *PersistedEntries) *ConfigMapController {
	cmLogger.Info("creating oidc store ConfigMap controller")
	return &ConfigMapController{
		apiClient:           apiClient,
		deploymentNamespace: deploymentNamespace,
		storePrefix:         storePrefix,
		store:               store,
		persistedEntries:    persistedEntries,
	}
}

// Init wires up the ConfigMap informer and event queue. Must be called before Start.
func (c *ConfigMapController) Init(ctx context.Context) {
	c.cmClient = kclient.NewFiltered[*corev1.ConfigMap](c.apiClient,
		kclient.Filter{
			ObjectFilter:  c.apiClient.ObjectFilter(),
			Namespace:     c.deploymentNamespace,
			LabelSelector: OidcStoreLabelSelector(c.storePrefix),
		})

	c.waitForSync = []cache.InformerSynced{
		c.cmClient.HasSynced,
		c.persistedEntries.entries.HasSynced,
		c.store.HasSynced,
	}

	c.oidcUpdates = c.store.SubscribeToUpdates()
	c.eventQueue = controllers.NewQueue("OidcStoreConfigMapController", controllers.WithReconciler(c.Reconcile), controllers.WithMaxAttempts(math.MaxInt), controllers.WithRateLimiter(rateLimiter))
}

// Start implements manager.Runnable.
func (c *ConfigMapController) Start(ctx context.Context) error {
	c.reconcileCtx = ctx

	cmLogger.Info("waiting for cache to sync")
	c.apiClient.Core().WaitForCacheSync(
		"kube oidc store ConfigMap syncer",
		ctx.Done(),
		c.waitForSync...,
	)

	cmLogger.Info("starting oidc store ConfigMap controller")
	persistedRegistration := c.persistedEntries.entries.Register(func(event krt.Event[PersistedEntry]) {
		c.enqueuePersistedEntry(event.Old)
		c.enqueuePersistedEntry(event.New)
	})
	defer persistedRegistration.UnregisterHandler()

	go func() {
		for {
			select {
			case u := <-c.oidcUpdates:
				for requestKey := range u {
					c.eventQueue.Add(requestQueueKey(c.deploymentNamespace, requestKey))
				}
			case <-ctx.Done():
				return
			}
		}
	}()
	go c.eventQueue.Run(ctx.Done())

	if !persistedRegistration.WaitUntilSynced(ctx.Done()) {
		return nil
	}

	<-ctx.Done()
	return nil
}

// Reconcile implements controllers.ReconcileFunc. It synchronizes the in-memory
// provider state for a single request key to the ConfigMap.
func (c *ConfigMapController) Reconcile(req types.NamespacedName) error {
	cmLogger.Debug("syncing oidc store to ConfigMap(s)")
	ctx := c.reconcileCtx
	if ctx == nil {
		ctx = context.Background()
	}
	requestKey := remotehttp.FetchKey(req.Name)
	plan := planConfigMapSync(requestKey, c.persistedEntries.entriesForRequestKey(requestKey), c.storePrefix, c.store.ProviderByRequestKey)

	if plan.provider != nil {
		if err := c.upsertConfigMap(ctx, req.Namespace, plan.upsertName, *plan.provider); err != nil {
			return err
		}
	}
	for _, deleteName := range plan.deleteNames {
		cmLogger.Debug("deleting ConfigMap", "name", deleteName)
		if err := client.IgnoreNotFound(c.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Delete(ctx, deleteName, metav1.DeleteOptions{})); err != nil {
			return err
		}
	}

	return nil
}

// NeedLeaderElection returns true; only the leader should write ConfigMaps.
func (c *ConfigMapController) NeedLeaderElection() bool {
	return true
}

func (c *ConfigMapController) newOidcStoreConfigMap(name string) *corev1.ConfigMap {
	return &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: c.deploymentNamespace,
			Labels:    OidcStoreConfigMapLabel(c.storePrefix),
		},
		Data: make(map[string]string),
	}
}

func (c *ConfigMapController) enqueuePersistedEntry(entry *PersistedEntry) {
	if entry == nil {
		return
	}
	requestKey, ok := entry.RequestKey()
	if !ok {
		return
	}
	c.eventQueue.Add(requestQueueKey(c.deploymentNamespace, requestKey))
}

func (c *ConfigMapController) upsertConfigMap(ctx context.Context, namespace, name string, provider DiscoveredProvider) error {
	existingCm := c.cmClient.Get(name, namespace)
	if existingCm == nil {
		cmLogger.Debug("creating ConfigMap", "name", name)
		newCm := c.newOidcStoreConfigMap(name)
		if err := SetProviderInConfigMap(newCm, provider); err != nil {
			cmLogger.Error("error setting provider in ConfigMap", "error", err)
			return err
		}

		if _, err := c.apiClient.Kube().CoreV1().ConfigMaps(namespace).Create(ctx, newCm, metav1.CreateOptions{}); err != nil {
			cmLogger.Error("error creating ConfigMap", "error", err)
			return err
		}
		return nil
	}

	cmLogger.Debug("updating ConfigMap", "name", name)
	if err := SetProviderInConfigMap(existingCm, provider); err != nil {
		cmLogger.Error("error updating ConfigMap", "error", err)
		return err
	}
	if _, err := c.apiClient.Kube().CoreV1().ConfigMaps(namespace).Update(ctx, existingCm, metav1.UpdateOptions{}); err != nil {
		cmLogger.Error("error updating oidc ConfigMap", "error", err)
		return err
	}
	return nil
}

func requestQueueKey(namespace string, requestKey remotehttp.FetchKey) types.NamespacedName {
	return types.NamespacedName{
		Namespace: namespace,
		Name:      string(requestKey),
	}
}

func planConfigMapSync(
	requestKey remotehttp.FetchKey,
	existingEntries []PersistedEntry,
	storePrefix string,
	lookup func(remotehttp.FetchKey) (DiscoveredProvider, bool),
) configMapSyncPlan {
	if provider, ok := lookup(requestKey); ok {
		canonicalName := OidcConfigMapName(storePrefix, provider.RequestKey)
		deleteNames := make([]string, 0, len(existingEntries))
		for _, existingEntry := range existingEntries {
			if existingEntry.NamespacedName.Name != canonicalName {
				deleteNames = append(deleteNames, existingEntry.NamespacedName.Name)
			}
		}
		return configMapSyncPlan{
			upsertName:  canonicalName,
			provider:    &provider,
			deleteNames: deleteNames,
		}
	}

	if len(existingEntries) == 0 {
		return configMapSyncPlan{}
	}

	deleteNames := make([]string, 0, len(existingEntries))
	for _, existingEntry := range existingEntries {
		deleteNames = append(deleteNames, existingEntry.NamespacedName.Name)
	}
	return configMapSyncPlan{deleteNames: deleteNames}
}
