package deployer_test

import (
	"context"
	"errors"
	"testing"

	"istio.io/istio/pkg/config/schema/gvk"
	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	_ "github.com/envoyproxy/go-control-plane/envoy/extensions/upstreams/http/v3"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient/fake"
	"github.com/kgateway-dev/kgateway/v2/pkg/deployer"
	deployerinternal "github.com/kgateway-dev/kgateway/v2/pkg/kgateway/deployer"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/schemes"
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

		d, err := deployerinternal.NewGatewayDeployer(
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
