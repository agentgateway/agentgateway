package agentgatewaysyncer

import (
	meshconfig "istio.io/api/mesh/v1alpha1"
	"istio.io/istio/pkg/config/mesh"
	corev1 "k8s.io/api/core/v1"
)

const (
	// IstioMeshConfigMapName is the default name of the ConfigMap that holds the Istio mesh config.
	IstioMeshConfigMapName = "istio"
	// IstioMeshConfigMapKey is the key in that ConfigMap containing the mesh YAML.
	IstioMeshConfigMapKey = "mesh"
)

// GetMeshConfigMapName returns the ConfigMap name for the Istio mesh config (e.g. "istio" or "istio-rev1").
func GetMeshConfigMapName(revision string) string {
	name := IstioMeshConfigMapName
	if revision == "" || revision == "default" {
		return name
	}
	return name + "-" + revision
}

// ParseMeshConfigFromConfigMap parses the Istio mesh YAML from the given ConfigMap (key "mesh")
// and returns the MeshConfig with defaults applied. Returns nil if the key is missing or parse fails.
func ParseMeshConfigFromConfigMap(cm *corev1.ConfigMap) *meshconfig.MeshConfig {
	if cm == nil {
		return nil
	}
	yamlStr, ok := cm.Data[IstioMeshConfigMapKey]
	if !ok || yamlStr == "" {
		return nil
	}
	mc, err := mesh.ApplyMeshConfig(yamlStr, mesh.DefaultMeshConfig())
	if err != nil {
		return nil
	}
	return mc
}
