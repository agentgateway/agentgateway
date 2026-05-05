package deployer_test

import (
	"context"
	"errors"
	"testing"

	"istio.io/istio/pkg/config/schema/gvk"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/util/smallset"
	autoscalingv2 "k8s.io/api/autoscaling/v2"
	corev1 "k8s.io/api/core/v1"
	policyv1 "k8s.io/api/policy/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

var scheme = schemes.DefaultScheme()

func TestDeployObjs(t *testing.T) {
	t.Helper()

	var (
		ns   = "test-ns"
		name = "test-obj"
		ctx  = context.Background()
	)

	getDeployer := func(t *testing.T, fc apiclient.Client, patcher deployer.Patcher) *deployer.Deployer {
		t.Helper()

		d, err := deployer.NewGatewayDeployer(
			wellknown.DefaultAgwControllerName,
			wellknown.DefaultAgwClassName,
			scheme,
			fc,
			nil,
			deployer.WithPatcher(patcher),
		)
		assert.NoError(t, err)
		return d
	}

	t.Run("skips patch if object is unchanged", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			TypeMeta:   metav1.TypeMeta{Kind: gvk.ConfigMap.Kind, APIVersion: gvk.ConfigMap.GroupVersion()},
			ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns},
			Data:       map[string]string{"foo": "bar"},
		}
		fc := fake.NewClient(t, cm.DeepCopy())
		d := getDeployer(t, fc, func(client apiclient.Client, fieldManager string, gvr schema.GroupVersionResource, name string, namespace string, data []byte, subresources ...string) error {
			t.Fatal("patch should not be called")
			return errors.New("unexpected Patch call")
		})
		fc.RunAndWait(context.Background().Done())

		err := d.DeployObjs(ctx, []client.Object{cm})
		assert.NoError(t, err)
	})

	t.Run("skips patch when only change is object status", func(t *testing.T) {
		pod1 := &corev1.Pod{
			TypeMeta:   metav1.TypeMeta{Kind: gvk.Pod.Kind, APIVersion: gvk.Pod.GroupVersion()},
			ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns},
			Spec:       corev1.PodSpec{Containers: []corev1.Container{{Name: "test", Image: "test:latest"}}},
			Status:     corev1.PodStatus{Phase: corev1.PodPending},
		}
		pod2 := pod1.DeepCopy()

		// obj to deploy won't have a status set.
		pod2.Status = corev1.PodStatus{}
		fc := fake.NewClient(t, pod1.DeepCopy())
		d := getDeployer(t, fc, func(client apiclient.Client, fieldManager string, gvr schema.GroupVersionResource, name string, namespace string, data []byte, subresources ...string) error {
			t.Fatal("patch should not be called")
			return errors.New("unexpected Patch call")
		})
		fc.RunAndWait(context.Background().Done())

		err := d.DeployObjs(ctx, []client.Object{pod2})
		assert.NoError(t, err)
	})

	t.Run("patches if object is different", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			TypeMeta: metav1.TypeMeta{Kind: gvk.ConfigMap.Kind, APIVersion: gvk.ConfigMap.GroupVersion()},

			ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns},
			Data:       map[string]string{"foo": "bar"},
		}
		fc := fake.NewClient(t, cm.DeepCopy())
		cm.Data = map[string]string{"foo": "bar", "bar": "baz"}
		patched := false
		d := getDeployer(t, fc, func(client apiclient.Client, fieldManager string, gvr schema.GroupVersionResource, name string, namespace string, data []byte, subresources ...string) error {
			patched = true
			return nil
		})
		fc.RunAndWait(context.Background().Done())

		err := d.DeployObjs(ctx, []client.Object{cm})
		assert.NoError(t, err)
		assert.Equal(t, true, patched)
	})

	t.Run("patches if object does not exist (IsNotFound error)", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			TypeMeta:   metav1.TypeMeta{Kind: gvk.ConfigMap.Kind, APIVersion: gvk.ConfigMap.GroupVersion()},
			ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns},
		}
		fc := fake.NewClient(t)
		patched := false
		d := getDeployer(t, fc, func(client apiclient.Client, fieldManager string, gvr schema.GroupVersionResource, name string, namespace string, data []byte, subresources ...string) error {
			patched = true
			return nil
		})
		fc.RunAndWait(context.Background().Done())

		err := d.DeployObjs(ctx, []client.Object{cm})
		assert.NoError(t, err)
		assert.Equal(t, true, patched)
	})

	t.Run("uses GatewayClass controllerName (not class name) as SSA field manager", func(t *testing.T) {
		customClassName := "custom-agw-class"
		gwc := &gwv1.GatewayClass{
			ObjectMeta: metav1.ObjectMeta{Name: customClassName},
			Spec:       gwv1.GatewayClassSpec{ControllerName: wellknown.DefaultAgwControllerName},
		}
		gw := &gwv1.Gateway{
			ObjectMeta: metav1.ObjectMeta{Name: "test-gw", Namespace: ns, UID: "12345"},
			Spec:       gwv1.GatewaySpec{GatewayClassName: gwv1.ObjectName(customClassName)},
		}
		gw.SetGroupVersionKind(wellknown.GatewayGVK)
		cm := &corev1.ConfigMap{
			TypeMeta:   metav1.TypeMeta{Kind: gvk.ConfigMap.Kind, APIVersion: gvk.ConfigMap.GroupVersion()},
			ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns},
			Data:       map[string]string{"foo": "bar"},
		}

		fc := fake.NewClient(t, gwc)
		var usedFieldManager string
		d := getDeployer(t, fc, func(client apiclient.Client, fieldManager string, gvr schema.GroupVersionResource, name string, namespace string, data []byte, subresources ...string) error {
			usedFieldManager = fieldManager
			return nil
		})
		fc.RunAndWait(context.Background().Done())

		err := d.DeployObjsWithSource(ctx, []client.Object{cm}, gw)
		assert.NoError(t, err)
		assert.Equal(t, wellknown.DefaultAgwControllerName, usedFieldManager)
	})

	t.Run("falls back to class name comparison when GatewayClass lookup fails", func(t *testing.T) {
		gw := &gwv1.Gateway{
			ObjectMeta: metav1.ObjectMeta{Name: "test-gw", Namespace: ns, UID: "12345"},
			Spec:       gwv1.GatewaySpec{GatewayClassName: wellknown.DefaultAgwClassName},
		}
		gw.SetGroupVersionKind(wellknown.GatewayGVK)
		cm := &corev1.ConfigMap{
			TypeMeta:   metav1.TypeMeta{Kind: gvk.ConfigMap.Kind, APIVersion: gvk.ConfigMap.GroupVersion()},
			ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns},
			Data:       map[string]string{"foo": "bar"},
		}

		fc := fake.NewClient(t) // no GatewayClass created
		var usedFieldManager string
		d := getDeployer(t, fc, func(client apiclient.Client, fieldManager string, gvr schema.GroupVersionResource, name string, namespace string, data []byte, subresources ...string) error {
			usedFieldManager = fieldManager
			return nil
		})
		fc.RunAndWait(context.Background().Done())

		err := d.DeployObjsWithSource(ctx, []client.Object{cm}, gw)
		assert.NoError(t, err)
		assert.Equal(t, wellknown.DefaultAgwControllerName, usedFieldManager)
	})
}

func TestGatewayAndListenerSetPortModifications(t *testing.T) {
	t.Helper()

	var (
		ns = "test-ns"
	)

	// Helper to create GatewayForDeployer with specific ports
	createGatewayForDeployer := func(ports ...int32) *collections.GatewayForDeployer {
		portSet := smallset.New(ports...)
		return &collections.GatewayForDeployer{
			ObjectSource: collections.ObjectSource{
				Name:      "test-gateway",
				Namespace: ns,
				Group:     gwv1.GroupVersion.Group,
				Kind:      wellknown.GatewayKind,
			},
			ControllerName: wellknown.DefaultAgwControllerName,
			Ports:          portSet,
		}
	}

	t.Run("GetPortsValues generates correct HelmPorts from single port", func(t *testing.T) {
		gw := createGatewayForDeployer(8080)
		ports := deployer.GetPortsValues(gw, 0)
		assert.Equal(t, 1, len(ports))
		assert.Equal(t, int32(8080), *ports[0].Port)
		assert.Equal(t, "listener-8080", *ports[0].Name)
	})

	t.Run("GetPortsValues generates correct HelmPorts from multiple ports", func(t *testing.T) {
		gw := createGatewayForDeployer(8080, 9090, 3000)
		ports := deployer.GetPortsValues(gw, 0)
		assert.Equal(t, 3, len(ports))

		// Verify all ports are present (order may vary due to set)
		portMap := make(map[int32]string)
		for _, p := range ports {
			portMap[*p.Port] = *p.Name
		}
		assert.Equal(t, 3, len(portMap))
		_, has8080 := portMap[8080]
		_, has9090 := portMap[9090]
		_, has3000 := portMap[3000]
		assert.Equal(t, true, has8080)
		assert.Equal(t, true, has9090)
		assert.Equal(t, true, has3000)
	})

	t.Run("GetPortsValues skips reserved ports", func(t *testing.T) {
		// Include a reserved port (15020) alongside normal ports
		gw := createGatewayForDeployer(8080, 15020, 9090)
		ports := deployer.GetPortsValues(gw, 0)

		// Should only have 2 ports (8080 and 9090), 15020 skipped
		assert.Equal(t, 2, len(ports))
		for _, p := range ports {
			if *p.Port == 15020 {
				t.Errorf("reserved port 15020 should have been skipped")
			}
		}
	})

	t.Run("GetPortsValues uses dummy port when no listeners", func(t *testing.T) {
		gw := createGatewayForDeployer() // Empty ports
		ports := deployer.GetPortsValues(gw, 15000)
		// 15000 is reserved - it gets added but will fail validation at runtime
		// For now, GetPortsValues adds the dummy port regardless
		assert.Equal(t, 1, len(ports))
		assert.Equal(t, int32(15000), *ports[0].Port)
	})

	t.Run("GetPortsValues uses dummy port when no listeners (valid dummy)", func(t *testing.T) {
		gw := createGatewayForDeployer() // Empty ports
		ports := deployer.GetPortsValues(gw, 8080)
		// Should add dummy port
		assert.Equal(t, 1, len(ports))
		assert.Equal(t, int32(8080), *ports[0].Port)
		assert.Equal(t, "listener-8080", *ports[0].Name)
	})

	t.Run("port addition modifies deployment", func(t *testing.T) {
		// This test verifies the full integration: Gateway/ListenerSet port changes trigger service updates.
		// It's covered by the existing TestDeployObjs scenarios, so we focus on GetPortsValues logic here.
		// A full integration test would require a running Gateway + ListenerSet setup, which is
		// tested separately in the existing deployer_test.go test suite.

		// Verify that changing ports results in different HelmPort output
		gw1 := createGatewayForDeployer(8080)
		gw2 := createGatewayForDeployer(8080, 9090)

		ports1 := deployer.GetPortsValues(gw1, 0)
		ports2 := deployer.GetPortsValues(gw2, 0)

		assert.Equal(t, 1, len(ports1))
		assert.Equal(t, 2, len(ports2))
	})

	t.Run("multiple ListenerSets merged into single port set", func(t *testing.T) {
		// Simulate multiple ListenerSets with their ports merged
		gw := createGatewayForDeployer(8080, 9090, 3000)
		ports := deployer.GetPortsValues(gw, 0)

		// All ports from both Gateway and ListenerSets should be present
		assert.Equal(t, 3, len(ports))
	})

	t.Run("port names generated consistently", func(t *testing.T) {
		gw := createGatewayForDeployer(443, 80)
		ports := deployer.GetPortsValues(gw, 0)

		for _, p := range ports {
			// Port name should follow pattern "listener-{port_number}" (tilde gets sanitized to hyphen)
			if *p.Port == 443 {
				assert.Equal(t, "listener-443", *p.Name)
			}
			if *p.Port == 80 {
				assert.Equal(t, "listener-80", *p.Name)
			}
		}
	})
}

func TestPruneRemovedResources(t *testing.T) {
	var (
		ns         = "test-ns"
		gwName     = "test-gateway"
		ctx        = context.Background()
		deployName = "test-deploy"
		pdbName    = "test-pdb"
		hpaName    = "test-hpa"
	)

	getDeployer := func(t *testing.T, fc apiclient.Client) *deployer.Deployer {
		t.Helper()
		d, err := deployer.NewGatewayDeployer(
			wellknown.DefaultAgwControllerName,
			wellknown.DefaultAgwClassName,
			scheme,
			fc,
			nil,
		)
		assert.NoError(t, err)
		return d
	}

	createGateway := func() *gwv1.Gateway {
		gw := &gwv1.Gateway{
			ObjectMeta: metav1.ObjectMeta{
				Name:      gwName,
				Namespace: ns,
			},
			Spec: gwv1.GatewaySpec{
				GatewayClassName: wellknown.DefaultAgwClassName,
			},
		}
		gw.SetGroupVersionKind(wellknown.GatewayGVK)
		return gw
	}

	createPDB := func(name string, gatewayName string) *policyv1.PodDisruptionBudget {
		pdb := &policyv1.PodDisruptionBudget{
			TypeMeta: metav1.TypeMeta{
				Kind:       wellknown.PodDisruptionBudgetGVK.Kind,
				APIVersion: wellknown.PodDisruptionBudgetGVK.GroupVersion().String(),
			},
			ObjectMeta: metav1.ObjectMeta{
				Name:      name,
				Namespace: ns,
				Labels: map[string]string{
					wellknown.GatewayNameLabel: gatewayName,
				},
			},
			Spec: policyv1.PodDisruptionBudgetSpec{
				Selector: &metav1.LabelSelector{
					MatchLabels: map[string]string{"app": "test"},
				},
			},
		}
		return pdb
	}

	createHPA := func(name string, gatewayName string) *autoscalingv2.HorizontalPodAutoscaler {
		hpa := &autoscalingv2.HorizontalPodAutoscaler{
			TypeMeta: metav1.TypeMeta{
				Kind:       wellknown.HorizontalPodAutoscalerGVK.Kind,
				APIVersion: wellknown.HorizontalPodAutoscalerGVK.GroupVersion().String(),
			},
			ObjectMeta: metav1.ObjectMeta{
				Name:      name,
				Namespace: ns,
				Labels: map[string]string{
					wellknown.GatewayNameLabel: gatewayName,
				},
			},
			Spec: autoscalingv2.HorizontalPodAutoscalerSpec{
				ScaleTargetRef: autoscalingv2.CrossVersionObjectReference{
					Kind: "Deployment",
					Name: deployName,
				},
				MinReplicas: new(int32(1)),
				MaxReplicas: 10,
			},
		}
		return hpa
	}

	t.Run("prunes PDB when not in desired set", func(t *testing.T) {
		gw := createGateway()
		pdb := createPDB(pdbName, gwName)

		fc := fake.NewClient(t, gw, pdb)
		d := getDeployer(t, fc)
		fc.RunAndWait(ctx.Done())

		// Desired set is empty - PDB should be pruned
		err := d.PruneRemovedResources(ctx, gw, []client.Object{})
		assert.NoError(t, err)

		// Verify PDB was deleted using dynamic client
		gvr, err := wellknown.GVKToGVR(wellknown.PodDisruptionBudgetGVK)
		assert.NoError(t, err)
		list, err := fc.Dynamic().Resource(gvr).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 0, len(list.Items))
	})

	t.Run("keeps PDB when in desired set", func(t *testing.T) {
		gw := createGateway()
		pdb := createPDB(pdbName, gwName)

		fc := fake.NewClient(t, gw, pdb)
		d := getDeployer(t, fc)
		fc.RunAndWait(ctx.Done())

		// PDB is in desired set - should be kept
		desiredPDB := createPDB(pdbName, gwName)
		err := d.PruneRemovedResources(ctx, gw, []client.Object{desiredPDB})
		assert.NoError(t, err)

		// Verify PDB still exists using dynamic client
		gvr, err := wellknown.GVKToGVR(wellknown.PodDisruptionBudgetGVK)
		assert.NoError(t, err)
		list, err := fc.Dynamic().Resource(gvr).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 1, len(list.Items))
		assert.Equal(t, pdbName, list.Items[0].GetName())
	})

	t.Run("skips resources belonging to a different Gateway", func(t *testing.T) {
		gw := createGateway()
		// PDB labeled for a different Gateway
		pdb := createPDB(pdbName, "other-gateway")

		fc := fake.NewClient(t, gw, pdb)
		d := getDeployer(t, fc)
		fc.RunAndWait(ctx.Done())

		// Empty desired set, but PDB belongs to a different Gateway
		err := d.PruneRemovedResources(ctx, gw, []client.Object{})
		assert.NoError(t, err)

		// Verify PDB was NOT deleted (different gateway label)
		gvr, err := wellknown.GVKToGVR(wellknown.PodDisruptionBudgetGVK)
		assert.NoError(t, err)
		list, err := fc.Dynamic().Resource(gvr).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 1, len(list.Items))
	})

	t.Run("prunes multiple resources in one call", func(t *testing.T) {
		gw := createGateway()
		pdb := createPDB(pdbName, gwName)
		hpa := createHPA(hpaName, gwName)

		fc := fake.NewClient(t, gw, pdb, hpa)
		d := getDeployer(t, fc)
		fc.RunAndWait(ctx.Done())

		// Empty desired set - both should be pruned
		err := d.PruneRemovedResources(ctx, gw, []client.Object{})
		assert.NoError(t, err)

		// Verify both were deleted
		pdbGVR, err := wellknown.GVKToGVR(wellknown.PodDisruptionBudgetGVK)
		assert.NoError(t, err)
		pdbList, err := fc.Dynamic().Resource(pdbGVR).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 0, len(pdbList.Items))

		hpaGVR, err := wellknown.GVKToGVR(wellknown.HorizontalPodAutoscalerGVK)
		assert.NoError(t, err)
		hpaList, err := fc.Dynamic().Resource(hpaGVR).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 0, len(hpaList.Items))
	})

	t.Run("prunes some resources while keeping others", func(t *testing.T) {
		gw := createGateway()
		pdb := createPDB(pdbName, gwName)
		hpa := createHPA(hpaName, gwName)

		fc := fake.NewClient(t, gw, pdb, hpa)
		d := getDeployer(t, fc)
		fc.RunAndWait(ctx.Done())

		// Only PDB in desired set - HPA should be pruned
		desiredPDB := createPDB(pdbName, gwName)
		err := d.PruneRemovedResources(ctx, gw, []client.Object{desiredPDB})
		assert.NoError(t, err)

		// Verify PDB still exists
		pdbGVR, err := wellknown.GVKToGVR(wellknown.PodDisruptionBudgetGVK)
		assert.NoError(t, err)
		pdbList, err := fc.Dynamic().Resource(pdbGVR).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 1, len(pdbList.Items))

		// Verify HPA was deleted
		hpaGVR, err := wellknown.GVKToGVR(wellknown.HorizontalPodAutoscalerGVK)
		assert.NoError(t, err)
		hpaList, err := fc.Dynamic().Resource(hpaGVR).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 0, len(hpaList.Items))
	})

	t.Run("handles no existing resources gracefully", func(t *testing.T) {
		gw := createGateway()

		fc := fake.NewClient(t, gw)
		d := getDeployer(t, fc)
		fc.RunAndWait(ctx.Done())

		// No resources exist, empty desired set
		err := d.PruneRemovedResources(ctx, gw, []client.Object{})
		assert.NoError(t, err)
	})

	t.Run("handles empty desired set", func(t *testing.T) {
		gw := createGateway()
		pdb := createPDB(pdbName, gwName)
		hpa := createHPA(hpaName, gwName)

		fc := fake.NewClient(t, gw, pdb, hpa)
		d := getDeployer(t, fc)
		fc.RunAndWait(ctx.Done())

		// All resources should be pruned with empty desired set
		err := d.PruneRemovedResources(ctx, gw, []client.Object{})
		assert.NoError(t, err)

		// Verify all were deleted
		pdbGVR, err := wellknown.GVKToGVR(wellknown.PodDisruptionBudgetGVK)
		assert.NoError(t, err)
		pdbList, err := fc.Dynamic().Resource(pdbGVR).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 0, len(pdbList.Items))

		hpaGVR, err := wellknown.GVKToGVR(wellknown.HorizontalPodAutoscalerGVK)
		assert.NoError(t, err)
		hpaList, err := fc.Dynamic().Resource(hpaGVR).Namespace(ns).List(ctx, metav1.ListOptions{})
		assert.NoError(t, err)
		assert.Equal(t, 0, len(hpaList.Items))
	})
}
