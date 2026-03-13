package oidcstore

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

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

var cmLogger = logging.New("oidc_provider_store_config_map_controller")

type OIDCStoreConfigMapsController struct {
	apiClient           apiclient.Client
	cmClient            kclient.Client[*corev1.ConfigMap]
	eventQueue          controllers.Queue
	providerUpdates     chan map[string]oidc.StoredProvider
	providerStore       *oidc.Store
	deploymentNamespace string
	storePrefix         string
	waitForSync         []cache.InformerSynced
}

var rateLimiter = workqueue.NewTypedMaxOfRateLimiter(
	workqueue.NewTypedItemExponentialFailureRateLimiter[any](500*time.Millisecond, 10*time.Second),
	&workqueue.TypedBucketRateLimiter[any]{Limiter: rate.NewLimiter(rate.Limit(10), 100)},
)

func NewOIDCStoreConfigMapsController(
	apiClient apiclient.Client,
	storePrefix string,
	deploymentNamespace string,
	providerStore *oidc.Store,
) *OIDCStoreConfigMapsController {
	cmLogger.Info("creating oidc provider store ConfigMap controller")
	return &OIDCStoreConfigMapsController{
		apiClient:           apiClient,
		deploymentNamespace: deploymentNamespace,
		storePrefix:         storePrefix,
		providerStore:       providerStore,
	}
}

func (c *OIDCStoreConfigMapsController) Init(ctx context.Context) {
	c.cmClient = kclient.NewFiltered[*corev1.ConfigMap](c.apiClient, kclient.Filter{
		ObjectFilter:  c.apiClient.ObjectFilter(),
		Namespace:     c.deploymentNamespace,
		LabelSelector: oidc.StoreLabelSelector(c.storePrefix),
	})

	c.waitForSync = []cache.InformerSynced{
		c.cmClient.HasSynced,
	}

	c.providerUpdates = c.providerStore.SubscribeToUpdates()
	c.eventQueue = controllers.NewQueue(
		"OIDCProviderStoreConfigMapController",
		controllers.WithReconciler(c.Reconcile),
		controllers.WithMaxAttempts(math.MaxInt),
		controllers.WithRateLimiter(rateLimiter),
	)
}

func (c *OIDCStoreConfigMapsController) Start(ctx context.Context) error {
	cmLogger.Info("waiting for cache to sync")
	c.apiClient.Core().WaitForCacheSync(
		"kube oidc provider store ConfigMap syncer",
		ctx.Done(),
		c.waitForSync...,
	)

	cmLogger.Info("starting oidc provider store ConfigMap controller")
	c.cmClient.AddEventHandler(controllers.FromEventHandler(func(o controllers.Event) {
		c.eventQueue.AddObject(o.Latest())
	}))

	go func() {
		for {
			select {
			case updates := <-c.providerUpdates:
				for resourceKey := range updates {
					c.eventQueue.AddObject(c.newProviderStoreConfigMap(
						oidc.ProviderConfigMapName(c.storePrefix, resourceKey),
					))
				}
			case <-ctx.Done():
				return
			}
		}
	}()
	go c.eventQueue.Run(ctx.Done())

	<-ctx.Done()
	return nil
}

func (c *OIDCStoreConfigMapsController) Reconcile(req types.NamespacedName) error {
	cmLogger.Debug("syncing oidc provider store to ConfigMap(s)")
	ctx := context.Background()

	resourceKey, provider, ok := c.providerStore.ProviderByConfigMapName(req.Name)
	if !ok {
		cmLogger.Debug("deleting ConfigMap", "name", req.Name)
		return client.IgnoreNotFound(
			c.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Delete(ctx, req.Name, metav1.DeleteOptions{}),
		)
	}

	existingCm := c.cmClient.Get(req.Name, req.Namespace)
	if existingCm == nil {
		cmLogger.Debug("creating ConfigMap", "name", req.Name)
		newCm := c.newProviderStoreConfigMap(oidc.ProviderConfigMapName(c.storePrefix, resourceKey))
		if err := oidc.SetProviderInConfigMap(newCm, provider); err != nil {
			cmLogger.Error("error updating ConfigMap", "error", err)
			return err
		}
		_, err := c.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Create(ctx, newCm, metav1.CreateOptions{})
		if err != nil {
			cmLogger.Error("error creating ConfigMap", "error", err)
			return err
		}
	} else {
		cmLogger.Debug("updating ConfigMap", "name", req.Name)
		if err := oidc.SetProviderInConfigMap(existingCm, provider); err != nil {
			cmLogger.Error("error updating ConfigMap", "error", err)
			return err
		}
		_, err := c.apiClient.Kube().CoreV1().ConfigMaps(req.Namespace).Update(ctx, existingCm, metav1.UpdateOptions{})
		if err != nil {
			cmLogger.Error("error updating oidc provider ConfigMap", "error", err)
			return err
		}
	}

	return nil
}

func (c *OIDCStoreConfigMapsController) NeedLeaderElection() bool {
	return true
}

func (c *OIDCStoreConfigMapsController) newProviderStoreConfigMap(name string) *corev1.ConfigMap {
	return &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: c.deploymentNamespace,
			Labels:    oidc.StoreConfigMapLabel(c.storePrefix),
		},
		Data: make(map[string]string),
	}
}
