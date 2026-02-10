package agentjwksstore

import (
	"context"
	"math"
	"time"

	"golang.org/x/time/rate"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	"k8s.io/client-go/util/workqueue"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/jwks"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
)

// JwksStoreConfigMapsController is responsible for reliable synchronisation of internal jwks store state (jwks uri + jwks) to ConfigMaps.
// A single ConfigMap is used to store all jwks for a given jwks url.

var cmLogger = logging.New("jwks_store_config_map_controller")

type JwksStoreConfigMapsController struct {
	apiClient           apiclient.Client
	cmClient            kclient.Client[*corev1.ConfigMap]
	eventQueue          controllers.Queue
	jwksUpdates         chan map[string]string
	jwksStore           *jwks.JwksStore
	deploymentNamespace string
	storePrefix         string
	waitForSync         []cache.InformerSynced
}

var (
	rateLimiter = workqueue.NewTypedMaxOfRateLimiter(
		workqueue.NewTypedItemExponentialFailureRateLimiter[any](500*time.Millisecond, 10*time.Second),
		// 10 qps, 100 bucket size.  This is only for retry speed and its only the overall factor (not per item)
		&workqueue.TypedBucketRateLimiter[any]{Limiter: rate.NewLimiter(rate.Limit(10), 100)},
	)
)

func NewJWKSStoreConfigMapsController(apiClient apiclient.Client, storePrefix, deploymentNamespace string, jwksStore *jwks.JwksStore) *JwksStoreConfigMapsController {
	cmLogger.Info("creating jwks store ConfigMap controller")
	return &JwksStoreConfigMapsController{
		apiClient:           apiClient,
		deploymentNamespace: deploymentNamespace,
		storePrefix:         storePrefix,
		jwksStore:           jwksStore,
	}
}

func (jcm *JwksStoreConfigMapsController) Init(ctx context.Context) {
	jcm.cmClient = kclient.NewFiltered[*corev1.ConfigMap](jcm.apiClient,
		kclient.Filter{
			ObjectFilter:  jcm.apiClient.ObjectFilter(),
			Namespace:     jcm.deploymentNamespace,
			LabelSelector: jwks.JwksStoreLabelSelector(jcm.storePrefix)})

	jcm.waitForSync = []cache.InformerSynced{
		jcm.cmClient.HasSynced,
	}

	jcm.jwksUpdates = jcm.jwksStore.SubscribeToUpdates()
	jcm.eventQueue = controllers.NewQueue("JwksStoreConfigMapController", controllers.WithReconciler(jcm.Reconcile), controllers.WithMaxAttempts(math.MaxInt), controllers.WithRateLimiter(rateLimiter))
}

func (jcm *JwksStoreConfigMapsController) Start(ctx context.Context) error {
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
				jcm.eventQueue.AddObject(o.Latest())
			}))

	go func() {
		for {
			select {
			case u := <-jcm.jwksUpdates:
				// TODO (dmitri-d) could add an interface for adding events directly to the queue
				// jwks store and cm controller are tightly coupled anyway, this indirection isn't very useful
				for uri := range u {
					jcm.eventQueue.AddObject(jcm.newJwksStoreConfigMap(jwks.JwksConfigMapName(jcm.storePrefix, uri)))
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

func (jcm *JwksStoreConfigMapsController) Reconcile(req types.NamespacedName) error {
	cmLogger.Debug("syncing jwks store to ConfigMap(s)")
	ctx := context.Background()

	uri, storedJwks, ok := jcm.jwksStore.JwksByConfigMapName(req.Name)
	if !ok {
		cmLogger.Debug("deleting ConfigMap", "name", req.Name)
		return client.IgnoreNotFound(jcm.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Delete(ctx, req.Name, metav1.DeleteOptions{}))
	}

	existingCm := jcm.cmClient.Get(req.Name, req.Namespace)
	if existingCm == nil {
		cmLogger.Debug("creating ConfigMap", "name", req.Name)
		newCm := jcm.newJwksStoreConfigMap(jwks.JwksConfigMapName(jcm.storePrefix, uri))
		if err := jwks.SetJwksInConfigMap(newCm, uri, storedJwks); err != nil {
			cmLogger.Error("error updating ConfigMap", "error", err)
			return err // should we skip retries as json serialization error won't go away?
		}

		_, err := jcm.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Create(ctx, newCm, metav1.CreateOptions{})
		if err != nil {
			cmLogger.Error("error creating ConfigMap", "error", err)
			return err
		}
	} else {
		cmLogger.Debug("updating ConfigMap", "name", req.Name)
		if err := jwks.SetJwksInConfigMap(existingCm, uri, storedJwks); err != nil {
			cmLogger.Error("error updating ConfigMap", "error", err)
			return err // should we skip retries as json serialization error won't go away?
		}
		_, err := jcm.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Update(ctx, existingCm, metav1.UpdateOptions{})
		if err != nil {
			cmLogger.Error("error updating jwks ConfigMap", "error", err)
			return err
		}
	}

	return nil
}

// runs on the leader only
func (jcm *JwksStoreConfigMapsController) NeedLeaderElection() bool {
	return true
}

func (jcm *JwksStoreConfigMapsController) newJwksStoreConfigMap(name string) *corev1.ConfigMap {
	return &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: jcm.deploymentNamespace,
			Labels:    jwks.JwksStoreConfigMapLabel(jcm.storePrefix),
		},
		Data: make(map[string]string),
	}
}
