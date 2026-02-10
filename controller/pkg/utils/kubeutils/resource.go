package kubeutils

import (
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
)

func NamespacedNameFrom(obj client.Object) types.NamespacedName {
	return types.NamespacedName{
		Namespace: obj.GetNamespace(),
		Name:      obj.GetName(),
	}
}

// ToUnstructured converts a typed Object to an Unstructured object
func ToUnstructured(obj client.Object) (*unstructured.Unstructured, error) {
	if u, ok := obj.(*unstructured.Unstructured); ok {
		return u, nil
	}

	raw, err := runtime.DefaultUnstructuredConverter.ToUnstructured(obj)
	if err != nil {
		return nil, err
	}

	u := &unstructured.Unstructured{Object: raw}
	// If the typed object has GVK set, preserve it
	if gvk := obj.GetObjectKind().GroupVersionKind(); !gvk.Empty() {
		u.SetGroupVersionKind(gvk)
	}
	return u, nil
}

// IsNamespacedGVK returns true if the GVK is namespaced
func IsNamespacedGVK(gvk schema.GroupVersionKind) bool {
	switch gvk {
	case wellknown.ClusterRoleBindingGVK, wellknown.ClusterRoleGVK:
		return false
	default:
		return true
	}
}
