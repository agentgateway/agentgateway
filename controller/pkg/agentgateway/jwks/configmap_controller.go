package jwks

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

// ConfigMapController synchronizes persisted JWKS keysets to ConfigMaps.

var cmLogger = logging.New("jwks_store_config_map_controller")

type ConfigMapController struct {
	apiClient           apiclient.Client
	cmClient            kclient.Client[*corev1.ConfigMap]
	eventQueue          controllers.Queue
	jwksUpdates         <-chan sets.Set[remotehttp.FetchKey]
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
		// 10 qps, 100 bucket size.  This is only for retry speed and its only the overall factor (not per item)
		&workqueue.TypedBucketRateLimiter[any]{Limiter: rate.NewLimiter(rate.Limit(10), 100)},
	)
)

type configMapSyncPlan struct {
	upsertName  string
	keyset      *Keyset
	deleteNames []string
}

func NewConfigMapController(apiClient apiclient.Client, storePrefix, deploymentNamespace string, store *Store, persistedEntries *PersistedEntries) *ConfigMapController {
	cmLogger.Info("creating jwks store ConfigMap controller")
	return &ConfigMapController{
		apiClient:           apiClient,
		deploymentNamespace: deploymentNamespace,
		storePrefix:         storePrefix,
		store:               store,
		persistedEntries:    persistedEntries,
	}
}

func (jcm *ConfigMapController) Init(ctx context.Context) {
	jcm.cmClient = kclient.NewFiltered[*corev1.ConfigMap](jcm.apiClient,
		kclient.Filter{
			ObjectFilter:  jcm.apiClient.ObjectFilter(),
			Namespace:     jcm.deploymentNamespace,
			LabelSelector: JwksStoreLabelSelector(jcm.storePrefix)})

	jcm.waitForSync = []cache.InformerSynced{
		jcm.cmClient.HasSynced,
		jcm.persistedEntries.entries.HasSynced,
		jcm.store.HasSynced,
	}

	jcm.jwksUpdates = jcm.store.SubscribeToUpdates()
	jcm.eventQueue = controllers.NewQueue("JwksStoreConfigMapController", controllers.WithReconciler(jcm.Reconcile), controllers.WithMaxAttempts(math.MaxInt), controllers.WithRateLimiter(rateLimiter))
}

func (jcm *ConfigMapController) Start(ctx context.Context) error {
	jcm.reconcileCtx = ctx

	cmLogger.Info("waiting for cache to sync")
	jcm.apiClient.Core().WaitForCacheSync(
		"kube jwks store ConfigMap syncer",
		ctx.Done(),
		jcm.waitForSync...,
	)

	cmLogger.Info("starting jwks store ConfigMap controller")
	persistedRegistration := jcm.persistedEntries.entries.Register(func(event krt.Event[PersistedEntry]) {
		jcm.enqueuePersistedEntry(event.Old)
		jcm.enqueuePersistedEntry(event.New)
	})
	defer persistedRegistration.UnregisterHandler()

	go func() {
		for {
			select {
			case u := <-jcm.jwksUpdates:
				for requestKey := range u {
					jcm.eventQueue.Add(requestQueueKey(jcm.deploymentNamespace, requestKey))
				}
			case <-ctx.Done():
				return
			}
		}
	}()
	go jcm.eventQueue.Run(ctx.Done())

	if !persistedRegistration.WaitUntilSynced(ctx.Done()) {
		return nil
	}

	<-ctx.Done()
	return nil
}

func (jcm *ConfigMapController) Reconcile(req types.NamespacedName) error {
	cmLogger.Debug("syncing jwks store to ConfigMap(s)")
	ctx := jcm.reconcileCtx
	if ctx == nil {
		ctx = context.Background()
	}
	requestKey := remotehttp.FetchKey(req.Name)
	plan := planConfigMapSync(requestKey, jcm.persistedEntries.entriesForRequestKey(requestKey), jcm.storePrefix, jcm.store.JwksByRequestKey)

	if plan.keyset != nil {
		if err := jcm.upsertConfigMap(ctx, req.Namespace, plan.upsertName, *plan.keyset); err != nil {
			return err
		}
	}
	for _, deleteName := range plan.deleteNames {
		cmLogger.Debug("deleting ConfigMap", "name", deleteName)
		if err := client.IgnoreNotFound(jcm.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Delete(ctx, deleteName, metav1.DeleteOptions{})); err != nil {
			return err
		}
	}

	return nil
}

// runs on the leader only
func (jcm *ConfigMapController) NeedLeaderElection() bool {
	return true
}

func (jcm *ConfigMapController) newJwksStoreConfigMap(name string) *corev1.ConfigMap {
	return &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: jcm.deploymentNamespace,
			Labels:    JwksStoreConfigMapLabel(jcm.storePrefix),
		},
		Data: make(map[string]string),
	}
}

func (jcm *ConfigMapController) enqueuePersistedEntry(entry *PersistedEntry) {
	if entry == nil {
		return
	}
	requestKey, ok := entry.RequestKey()
	if !ok {
		return
	}
	jcm.eventQueue.Add(requestQueueKey(jcm.deploymentNamespace, requestKey))
}

func (jcm *ConfigMapController) upsertConfigMap(ctx context.Context, namespace, name string, keyset Keyset) error {
	existingCm := jcm.cmClient.Get(name, namespace)
	if existingCm == nil {
		cmLogger.Debug("creating ConfigMap", "name", name)
		newCm := jcm.newJwksStoreConfigMap(name)
		if err := SetJwksInConfigMap(newCm, keyset); err != nil {
			cmLogger.Error("error updating ConfigMap", "error", err)
			return err
		}

		if _, err := jcm.apiClient.Kube().CoreV1().ConfigMaps(namespace).Create(ctx, newCm, metav1.CreateOptions{}); err != nil {
			cmLogger.Error("error creating ConfigMap", "error", err)
			return err
		}
		return nil
	}

	cmLogger.Debug("updating ConfigMap", "name", name)
	if err := SetJwksInConfigMap(existingCm, keyset); err != nil {
		cmLogger.Error("error updating ConfigMap", "error", err)
		return err
	}
	if _, err := jcm.apiClient.Kube().CoreV1().ConfigMaps(namespace).Update(ctx, existingCm, metav1.UpdateOptions{}); err != nil {
		cmLogger.Error("error updating jwks ConfigMap", "error", err)
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
	lookup func(remotehttp.FetchKey) (Keyset, bool),
) configMapSyncPlan {
	if keyset, ok := lookup(requestKey); ok {
		canonicalName := JwksConfigMapName(storePrefix, keyset.RequestKey)
		deleteNames := make([]string, 0, len(existingEntries))
		for _, existingEntry := range existingEntries {
			// Clean up any non-canonical persisted entries for this request key,
			// including legacy ConfigMaps from pre-migration naming.
			if existingEntry.NamespacedName.Name != canonicalName {
				deleteNames = append(deleteNames, existingEntry.NamespacedName.Name)
			}
		}
		return configMapSyncPlan{
			upsertName:  canonicalName,
			keyset:      &keyset,
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
