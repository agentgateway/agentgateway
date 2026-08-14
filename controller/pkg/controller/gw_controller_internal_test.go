package controller

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/config/schema/gvr"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/test"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
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
		ObjectMeta: metav1.ObjectMeta{
			Name:       "gw",
			Namespace:  namespace,
			Generation: 7,
		},
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
		ObjectMeta: metav1.ObjectMeta{Name: wellknown.DefaultAgwClassName},
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
		ObjectMeta: metav1.ObjectMeta{Name: "class-params", Namespace: namespace},
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: overlay,
			},
		},
	}
	gatewayParams := &agentgateway.AgentgatewayParameters{
		ObjectMeta: metav1.ObjectMeta{Name: "gateway-params", Namespace: namespace},
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
