package agentgatewaysyncer

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
)

func TestGetMeshConfigMapName(t *testing.T) {
	tests := []struct {
		name     string
		revision string
		want     string
	}{
		{"empty revision", "", IstioMeshConfigMapName},
		{"default revision", "default", IstioMeshConfigMapName},
		{"custom revision", "rev1", IstioMeshConfigMapName + "-rev1"},
		{"another revision", "canary", IstioMeshConfigMapName + "-canary"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := GetMeshConfigMapName(tt.revision)
			assert.Equal(t, tt.want, got)
		})
	}
}

func TestParseMeshConfigFromConfigMap(t *testing.T) {
	t.Run("nil ConfigMap", func(t *testing.T) {
		got := ParseMeshConfigFromConfigMap(nil)
		assert.Nil(t, got)
	})

	t.Run("missing key", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			Data: map[string]string{
				"other-key": "value",
			},
		}
		got := ParseMeshConfigFromConfigMap(cm)
		assert.Nil(t, got)
	})

	t.Run("empty key value", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			Data: map[string]string{
				IstioMeshConfigMapKey: "",
			},
		}
		got := ParseMeshConfigFromConfigMap(cm)
		assert.Nil(t, got)
	})

	t.Run("invalid YAML", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			Data: map[string]string{
				IstioMeshConfigMapKey: "trustDomain: [unclosed",
			},
		}
		got := ParseMeshConfigFromConfigMap(cm)
		assert.Nil(t, got)
	})

	t.Run("valid YAML with trustDomain", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			Data: map[string]string{
				IstioMeshConfigMapKey: "trustDomain: my-custom-trust.domain\n",
			},
		}
		got := ParseMeshConfigFromConfigMap(cm)
		require.NotNil(t, got)
		assert.Equal(t, "my-custom-trust.domain", got.TrustDomain)
	})
}
