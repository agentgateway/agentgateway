package meshconfig

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

type alwaysSynced struct{}

func (alwaysSynced) WaitUntilSynced(stop <-chan struct{}) bool { return true }
func (alwaysSynced) HasSynced() bool                           { return true }

func cm(ns, name, mesh string) *corev1.ConfigMap {
	return &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Namespace: ns, Name: name},
		Data:       map[string]string{IstioMeshConfigMapKey: mesh},
	}
}

// TestNewSingletonTrustDomain verifies the singleton selects the mesh ConfigMap by
// namespace/revision and surfaces its trust domain, defaulting to the Istio default
// ("cluster.local") when the ConfigMap is absent or doesn't set one.
func TestNewSingletonTrustDomain(t *testing.T) {
	tests := []struct {
		name     string
		ns       string
		revision string
		cms      []*corev1.ConfigMap
		want     string
	}{
		{
			name:     "reads trustDomain from the default istio configmap",
			ns:       "istio-system",
			revision: "default",
			cms:      []*corev1.ConfigMap{cm("istio-system", "istio", "trustDomain: cluster1")},
			want:     "cluster1",
		},
		{
			name:     "reads trustDomain from non-default namespace",
			ns:       "istio-1-30-system",
			revision: "1-30",
			cms:      []*corev1.ConfigMap{cm("istio-1-30-system", "istio-1-30", "trustDomain: cluster1")},
			want:     "cluster1",
		},
		{
			name:     "reads from the revisioned configmap",
			ns:       "istio-system",
			revision: "1-30",
			cms:      []*corev1.ConfigMap{cm("istio-system", "istio-1-30", "trustDomain: cluster1")},
			want:     "cluster1",
		},
		{
			name:     "configmap present without trustDomain uses the istio default",
			ns:       "istio-system",
			revision: "default",
			cms:      []*corev1.ConfigMap{cm("istio-system", "istio", "{}")},
			want:     "cluster.local",
		},
		{
			name:     "absent configmap uses the istio default",
			ns:       "istio-system",
			revision: "default",
			cms:      nil,
			want:     "cluster.local",
		},
		{
			name:     "wrong revision misses the configmap and uses the istio default",
			ns:       "istio-system",
			revision: "1-30",
			cms:      []*corev1.ConfigMap{cm("istio-system", "istio", "trustDomain: cluster1")},
			want:     "cluster.local",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			stop := test.NewStop(t)
			opts := krtutil.NewKrtOptions(stop, new(krt.DebugHandler))
			cms := krt.NewStaticCollection(alwaysSynced{}, tt.cms, opts.ToOptions("test/ConfigMaps")...)
			mc := NewSingleton(cms, tt.ns, tt.revision, opts.ToOptions("test/MeshConfig")...)
			mc.AsCollection().WaitUntilSynced(stop)
			got := mc.Get()
			require.NotNil(t, got)
			require.NotNil(t, got.MeshConfig)
			assert.Equal(t, tt.want, got.MeshConfig.GetTrustDomain())
		})
	}
}
