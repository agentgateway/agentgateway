package jwks

import (
	"context"
	"math"
	"time"

	"golang.org/x/time/rate"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	"k8s.io/client-go/util/workqueue"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

// ConfigMapController synchronizes persisted JWKS artifacts to ConfigMaps.

var cmLogger = logging.New("jwks_store_config_map_controller")

type ConfigMapController struct {
	apiClient           apiclient.Client
	cmClient            kclient.Client[*corev1.ConfigMap]
	eventQueue          controllers.Queue
	jwksUpdates         <-chan map[RequestKey]struct{}
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
	artifact    *Artifact
	deleteNames []string
}

func NewConfigMapController(apiClient apiclient.Client, storePrefix, deploymentNamespace string, store *Store) *ConfigMapController {
	cmLogger.Info("creating jwks store ConfigMap controller")
	return &ConfigMapController{
		apiClient:           apiClient,
		deploymentNamespace: deploymentNamespace,
		storePrefix:         storePrefix,
		store:               store,
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
	jcm.cmClient.AddEventHandler(
		controllers.FromEventHandler(
			func(o controllers.Event) {
				cm := controllers.Extract[*corev1.ConfigMap](o.Latest())
				if cm == nil {
					return
				}
				requestKey, err := RequestKeyFromConfigMap(cm)
				if err != nil {
					cmLogger.Debug("ignoring jwks ConfigMap event without a readable artifact", "name", cm.Name, "error", err)
					return
				}
				jcm.eventQueue.Add(requestQueueKey(jcm.deploymentNamespace, requestKey))
			}))
	jcm.enqueueExistingConfigMaps()

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

	<-ctx.Done()
	return nil
}

func (jcm *ConfigMapController) Reconcile(req types.NamespacedName) error {
	cmLogger.Debug("syncing jwks store to ConfigMap(s)")
	ctx := jcm.reconcileCtx
	if ctx == nil {
		ctx = context.Background()
	}
	requestKey := RequestKey(req.Name)
	plan := planConfigMapSync(requestKey, jcm.existingConfigMapsForRequestKey(req.Namespace, requestKey), jcm.storePrefix, jcm.store.JwksByRequestKey)

	if plan.artifact != nil {
		if err := jcm.upsertConfigMap(ctx, req.Namespace, plan.upsertName, *plan.artifact); err != nil {
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

func (jcm *ConfigMapController) enqueueExistingConfigMaps() {
	for _, cm := range jcm.cmClient.List(jcm.deploymentNamespace, labels.Everything()) {
		requestKey, err := RequestKeyFromConfigMap(cm)
		if err != nil {
			cmLogger.Debug("ignoring persisted jwks ConfigMap without a readable artifact", "name", cm.Name, "error", err)
			continue
		}
		jcm.eventQueue.Add(requestQueueKey(jcm.deploymentNamespace, requestKey))
	}
}

func (jcm *ConfigMapController) existingConfigMapsForRequestKey(namespace string, requestKey RequestKey) []*corev1.ConfigMap {
	var matches []*corev1.ConfigMap
	for _, cm := range jcm.cmClient.List(namespace, labels.Everything()) {
		storedRequestKey, err := RequestKeyFromConfigMap(cm)
		if err != nil {
			cmLogger.Debug("ignoring persisted jwks ConfigMap without a readable artifact", "name", cm.Name, "error", err)
			continue
		}
		if storedRequestKey == requestKey {
			matches = append(matches, cm)
		}
	}
	return matches
}

func (jcm *ConfigMapController) upsertConfigMap(ctx context.Context, namespace, name string, artifact Artifact) error {
	existingCm := jcm.cmClient.Get(name, namespace)
	if existingCm == nil {
		cmLogger.Debug("creating ConfigMap", "name", name)
		newCm := jcm.newJwksStoreConfigMap(name)
		if err := SetJwksInConfigMap(newCm, artifact); err != nil {
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
	if err := SetJwksInConfigMap(existingCm, artifact); err != nil {
		cmLogger.Error("error updating ConfigMap", "error", err)
		return err
	}
	if _, err := jcm.apiClient.Kube().CoreV1().ConfigMaps(namespace).Update(ctx, existingCm, metav1.UpdateOptions{}); err != nil {
		cmLogger.Error("error updating jwks ConfigMap", "error", err)
		return err
	}
	return nil
}

func requestQueueKey(namespace string, requestKey RequestKey) types.NamespacedName {
	return types.NamespacedName{
		Namespace: namespace,
		Name:      string(requestKey),
	}
}

func planConfigMapSync(
	requestKey RequestKey,
	existingCms []*corev1.ConfigMap,
	storePrefix string,
	lookup func(RequestKey) (Artifact, bool),
) configMapSyncPlan {
	if artifact, ok := lookup(requestKey); ok {
		canonicalName := JwksConfigMapName(storePrefix, artifact.RequestKey)
		deleteNames := make([]string, 0, len(existingCms))
		for _, existingCm := range existingCms {
			if existingCm.Name != canonicalName {
				deleteNames = append(deleteNames, existingCm.Name)
			}
		}
		return configMapSyncPlan{
			upsertName:  canonicalName,
			artifact:    &artifact,
			deleteNames: deleteNames,
		}
	}

	if len(existingCms) == 0 {
		return configMapSyncPlan{}
	}

	deleteNames := make([]string, 0, len(existingCms))
	for _, existingCm := range existingCms {
		deleteNames = append(deleteNames, existingCm.Name)
	}
	return configMapSyncPlan{deleteNames: deleteNames}
}
