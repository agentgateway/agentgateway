package fake

import (
	"context"
	"fmt"

	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient/clienttest"
	"istio.io/istio/pkg/test"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/gateway-api/pkg/consts"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/client/clientset/versioned"
	"github.com/kgateway-dev/kgateway/v2/pkg/client/clientset/versioned/fake"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/schemes"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

var _ apiclient.Client = (*cli)(nil)

type cli struct {
	kube.Client
	kgateway versioned.Interface
}

func NewClient(t test.Failer, objects ...client.Object) *cli {
	return NewClientWithExtraGVRs(t, nil, objects...)
}

func NewClientWithExtraGVRs(t test.Failer, extraGVRs []schema.GroupVersionResource, objects ...client.Object) *cli {
	known, kgw := filterObjects(objects...)
	c := &cli{
		Client:   fakeIstioClient(known...),
		kgateway: fakeKgwClient(kgw...),
	}

	allCRDs := append(testutils.AllCRDs, extraGVRs...)
	for _, crd := range allCRDs {
		clienttest.MakeCRDWithAnnotations(t, c.Client, crd, map[string]string{
			consts.BundleVersionAnnotation: consts.BundleVersion,
		})
	}

	apiclient.RegisterTypes()

	return c
}

func (c *cli) Kgateway() versioned.Interface {
	return c.kgateway
}

func (c *cli) Core() kube.Client {
	return c.Client
}

func fakeIstioClient(objects ...client.Object) kube.Client {
	c := kube.NewFakeClient(testutils.ToRuntimeObjects(objects...)...)
	// Also add to the Dynamic store
	for _, obj := range objects {
		nn := kubeutils.NamespacedNameFrom(obj)
		gvr := mustGetGVR(obj, kube.IstioScheme)
		d := c.Dynamic().Resource(gvr).Namespace(obj.GetNamespace())
		us, err := kubeutils.ToUnstructured(obj)
		if err != nil {
			panic(fmt.Sprintf("failed to convert to unstructured for object %T %s: %v", obj, nn, err))
		}
		_, err = d.Create(context.Background(), us, metav1.CreateOptions{})
		if err != nil {
			panic(fmt.Sprintf("failed to create in dynamic client for object %T %s: %v", obj, nn, err))
		}
	}

	return c
}

func fakeKgwClient(objects ...client.Object) *fake.Clientset {
	f := fake.NewSimpleClientset()
	for _, obj := range objects {
		gvr := mustGetGVR(obj, schemes.DefaultScheme())
		// Run Create() instead of Add(), so we can pass the GVR. Otherwise, Kubernetes guesses, and it guesses wrong for 'GatewayParameters'.
		// DeepCopy since it will mutate the managed fields/etc
		if err := f.Tracker().Create(gvr, obj.DeepCopyObject(), obj.(metav1.ObjectMetaAccessor).GetObjectMeta().GetNamespace()); err != nil {
			panic("failed to create: " + err.Error())
		}
	}
	return f
}

func filterObjects(objects ...client.Object) (istio []client.Object, kgw []client.Object) {
	for _, obj := range objects {
		switch obj.(type) {
		case
			*agentgateway.AgentgatewayPolicy,
			*agentgateway.AgentgatewayBackend,
			*agentgateway.AgentgatewayParameters:
			kgw = append(kgw, obj)
		default:
			istio = append(istio, obj)
		}
	}
	return istio, kgw
}

func mustGetGVR(obj client.Object, scheme *runtime.Scheme) schema.GroupVersionResource {
	gvr, err := getGVR(obj, scheme)
	if err != nil {
		panic(err)
	}
	return gvr
}

func getGVR(obj client.Object, scheme *runtime.Scheme) (schema.GroupVersionResource, error) {
	gvk := obj.GetObjectKind().GroupVersionKind()
	if gvk.Group == "" {
		gvks, _, _ := scheme.ObjectKinds(obj)
		gvk = gvks[0]
	}
	gvr, err := wellknown.GVKToGVR(gvk)
	if err != nil {
		// try unsafe guess
		gvr, _ = meta.UnsafeGuessKindToResource(gvk)
		if gvr == (schema.GroupVersionResource{}) {
			return schema.GroupVersionResource{}, fmt.Errorf("failed to get GVR for object %s: %v", kubeutils.NamespacedNameFrom(obj), err)
		}
	}
	if gvr.Group == "core" {
		gvr.Group = ""
	}
	return gvr, nil
}
