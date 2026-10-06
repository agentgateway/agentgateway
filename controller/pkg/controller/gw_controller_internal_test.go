package controller

import (
	"errors"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/config/schema/gvr"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	k8sfake "k8s.io/client-go/kubernetes/fake"
	k8stesting "k8s.io/client-go/testing"
	"k8s.io/client-go/tools/cache"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/reports"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestGatewayReconciler_InvalidWorkloadOverlaySetsInvalidParameters(t *testing.T) {
	const namespace = "default"
	paramsNamespace := gwv1.Namespace(namespace)
	overlay := &agentgateway.KubernetesResourceOverlay{
		Spec: &apiextensionsv1.JSON{Raw: []byte(`{"replicas": 2}`)},
	}
	gw := &gwv1.Gateway{
		Name:       "gw",
		Namespace:  namespace,
		Generation: 7,
		Spec: gwv1.GatewaySpec{
			GatewayClassName: gwv1.ObjectName(wellknown.DefaultAgwClassName),
			Infrastructure: &gwv1.GatewayInfrastructure{
				ParametersRef: &gwv1.LocalParametersReference{
					Group: agentgateway.GroupName,
					Kind:  gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind),
					Name:  "gateway-params",
				},
			},
		},
	}
	gwc := &gwv1.GatewayClass{
		Name: wellknown.DefaultAgwClassName,
		Spec: gwv1.GatewayClassSpec{
			ControllerName: gwv1.GatewayController(wellknown.DefaultAgwControllerName),
			ParametersRef: &gwv1.ParametersReference{
				Group:     agentgateway.GroupName,
				Kind:      gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind),
				Name:      "class-params",
				Namespace: &paramsNamespace,
			},
		},
	}
	classParams := &agentgateway.AgentgatewayParameters{
		Name: "class-params", Namespace: namespace,
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: overlay,
			},
		},
	}
	gatewayParams := &agentgateway.AgentgatewayParameters{
		Name: "gateway-params", Namespace: namespace,
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Workload: &agentgateway.AgentgatewayParametersWorkload{
					Kind: agentgateway.AgentgatewayParametersWorkloadDaemonSet,
				},
			},
		},
	}
	fakeClient := fake.NewClient(t, gw, gwc, classParams, gatewayParams)
	gwParams := deployer.NewGatewayParameters(fakeClient, &deployer.Inputs{})
	d, err := deployer.NewGatewayDeployer(
		wellknown.DefaultAgwControllerName,
		wellknown.DefaultAgwClassName,
		schemes.DefaultScheme(),
		fakeClient,
		gwParams,
	)
	require.NoError(t, err)
	filter := kclient.Filter{ObjectFilter: fakeClient.ObjectFilter()}
	reconciler := &gatewayReconciler{
		deployer:          d,
		gwParams:          gwParams,
		agwControllerName: wellknown.DefaultAgwControllerName,
		gwClient:          kclient.NewFilteredDelayed[*gwv1.Gateway](fakeClient, gvr.KubernetesGateway, filter),
		gwClassClient:     kclient.NewFilteredDelayed[*gwv1.GatewayClass](fakeClient, gvr.GatewayClass, filter),
	}
	stop := test.NewStop(t)
	fakeClient.RunAndWait(stop)
	hasSynced := []cache.InformerSynced{
		reconciler.gwClient.HasSynced,
		reconciler.gwClassClient.HasSynced,
	}
	for _, handler := range gwParams.GetCacheSyncHandlers() {
		hasSynced = append(hasSynced, handler)
	}
	kube.WaitForCacheSync("test-gateway-reconciler", stop, hasSynced...)

	err = reconciler.Reconcile(types.NamespacedName{Name: gw.Name, Namespace: gw.Namespace})
	require.Error(t, err)
	var accepted *metav1.Condition
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		updated := reconciler.gwClient.Get(gw.Name, gw.Namespace)
		require.NotNil(c, updated)
		accepted = meta.FindStatusCondition(updated.Status.Conditions, string(gwv1.GatewayConditionAccepted))
		assert.NotNil(c, accepted)
	}, time.Second, 10*time.Millisecond)
	require.NotNil(t, accepted)
	assert.Equal(t, metav1.ConditionFalse, accepted.Status)
	assert.Equal(t, string(gwv1.GatewayReasonInvalidParameters), accepted.Reason)
	assert.Equal(t, gw.Generation, accepted.ObservedGeneration)
	assert.Contains(t, accepted.Message, "deployment")
	assert.Contains(t, accepted.Message, "DaemonSet")
}

func TestGatewayReconciler_SessionKeySecretFailureSetsDeploymentFailed(t *testing.T) {
	gw := &gwv1.Gateway{
		Name:       "gw",
		Namespace:  "default",
		Generation: 3,
		Spec: gwv1.GatewaySpec{
			GatewayClassName: gwv1.ObjectName(wellknown.DefaultAgwClassName),
			Listeners: []gwv1.Listener{{
				Name:     "http",
				Port:     8080,
				Protocol: gwv1.HTTPProtocolType,
			}},
		},
	}
	gwc := &gwv1.GatewayClass{
		Name: wellknown.DefaultAgwClassName,
		Spec: gwv1.GatewayClassSpec{
			ControllerName: gwv1.GatewayController(wellknown.DefaultAgwControllerName),
		},
	}
	fakeClient := fake.NewClient(t, gw, gwc)
	fakeClient.Kube().(*k8sfake.Clientset).PrependReactor("create", "secrets",
		func(k8stesting.Action) (bool, runtime.Object, error) {
			return true, nil, errors.New("api server unavailable")
		})
	gwParams := deployer.NewGatewayParameters(fakeClient, &deployer.Inputs{
		ImageDefaults: &agentgateway.Image{
			Registry:   new("cr.agentgateway.dev"),
			Repository: new("agentgateway"),
			Tag:        new("latest"),
		},
		ControlPlane: deployer.ControlPlaneInfo{
			XdsHost:          "agentgateway",
			AgwXdsPort:       15000,
			XdsTLSSecretName: "xds-tls",
			ControlPlaneNs:   "agentgateway-system",
		},
		NoListenersDummyPort:       15021,
		AgentgatewayClassName:      wellknown.DefaultAgwClassName,
		AgentgatewayControllerName: wellknown.DefaultAgwControllerName,
		AgwCollections: &agwplugins.AgwCollections{
			ControllerName:      wellknown.DefaultAgwControllerName,
			GatewaysForDeployer: krt.NewStaticCollection[collections.GatewayForDeployer](nil, nil),
		},
	})
	d, err := deployer.NewGatewayDeployer(
		wellknown.DefaultAgwControllerName,
		wellknown.DefaultAgwClassName,
		schemes.DefaultScheme(),
		fakeClient,
		gwParams,
	)
	require.NoError(t, err)
	filter := kclient.Filter{ObjectFilter: fakeClient.ObjectFilter()}
	reconciler := &gatewayReconciler{
		deployer:          d,
		gwParams:          gwParams,
		agwControllerName: wellknown.DefaultAgwControllerName,
		gwClient:          kclient.NewFilteredDelayed[*gwv1.Gateway](fakeClient, gvr.KubernetesGateway, filter),
		gwClassClient:     kclient.NewFilteredDelayed[*gwv1.GatewayClass](fakeClient, gvr.GatewayClass, filter),
	}
	stop := test.NewStop(t)
	fakeClient.RunAndWait(stop)
	hasSynced := []cache.InformerSynced{
		reconciler.gwClient.HasSynced,
		reconciler.gwClassClient.HasSynced,
	}
	for _, handler := range gwParams.GetCacheSyncHandlers() {
		hasSynced = append(hasSynced, handler)
	}
	kube.WaitForCacheSync("test-gateway-reconciler", stop, hasSynced...)

	err = reconciler.Reconcile(types.NamespacedName{Name: gw.Name, Namespace: gw.Namespace})
	require.ErrorIs(t, err, deployer.ErrSessionKey)
	var programmed *metav1.Condition
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		updated := reconciler.gwClient.Get(gw.Name, gw.Namespace)
		require.NotNil(c, updated)
		programmed = meta.FindStatusCondition(updated.Status.Conditions, string(gwv1.GatewayConditionProgrammed))
		assert.NotNil(c, programmed)
	}, time.Second, 10*time.Millisecond)
	require.NotNil(t, programmed)
	assert.Equal(t, metav1.ConditionFalse, programmed.Status)
	assert.Equal(t, string(reports.GatewayReasonDeploymentFailed), programmed.Reason)
	assert.Equal(t, gw.Generation, programmed.ObservedGeneration)
	assert.Contains(t, programmed.Message, "api server unavailable")

	updated := reconciler.gwClient.Get(gw.Name, gw.Namespace)
	require.NotNil(t, updated)
	assert.Nil(t, meta.FindStatusCondition(updated.Status.Conditions, string(gwv1.GatewayConditionAccepted)))
}
