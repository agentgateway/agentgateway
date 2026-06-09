package meshconfig

import (
	"istio.io/istio/pilot/pkg/serviceregistry/ambient"
	"istio.io/istio/pkg/config/mesh"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
)

// NewSingleton builds a krt singleton tracking the revisioned Istio mesh config
// with standard fallbacks (non-revisioned "istio" cm, static Istio defaults).
func NewSingleton(
	configMaps krt.Collection[*corev1.ConfigMap],
	namespace,
	revision string,
	opts ...krt.CollectionOption,
) krt.Singleton[ambient.MeshConfig] {
	name := GetMeshConfigMapName(revision)
	return krt.NewSingleton(func(ctx krt.HandlerContext) *ambient.MeshConfig {
		cm := krt.FetchOne(ctx, configMaps, krt.FilterObjectName(types.NamespacedName{Namespace: namespace, Name: name}))
		if mc := ParseMeshConfigFromConfigMap(ptr.Flatten(cm)); mc != nil {
			return &ambient.MeshConfig{MeshConfig: mc}
		}
		return &ambient.MeshConfig{MeshConfig: mesh.DefaultMeshConfig()}
	}, opts...)
}
