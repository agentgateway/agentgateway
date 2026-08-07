package syncer

import (
	"testing"

	"istio.io/istio/pkg/workloadapi"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
)

func TestToInferencePoolAppProtocol(t *testing.T) {
	testCases := []struct {
		name        string
		appProtocol inf.AppProtocol
		want        workloadapi.AppProtocol
	}{
		{
			name:        "h2c",
			appProtocol: inf.AppProtocolH2C,
			want:        workloadapi.AppProtocol_HTTP2,
		},
		{
			name:        "http",
			appProtocol: inf.AppProtocolHTTP,
			want:        workloadapi.AppProtocol_HTTP11,
		},
		{
			name: "unspecified",
			want: workloadapi.AppProtocol_HTTP11,
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			got := toInferencePoolAppProtocol(tc.appProtocol)
			if got != tc.want {
				t.Fatalf("toInferencePoolAppProtocol(%q) = %v, want %v", tc.appProtocol, got, tc.want)
			}
		})
	}
}

func TestInferencePoolBuilderPropagatesH2CAppProtocol(t *testing.T) {
	pool := &inf.InferencePool{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "models",
			Namespace: "default",
		},
		Spec: inf.InferencePoolSpec{
			Selector: inf.LabelSelector{
				MatchLabels: map[inf.LabelKey]inf.LabelValue{
					"app": "models",
				},
			},
			TargetPorts: []inf.Port{
				{Number: 8000},
				{Number: 8001},
			},
			AppProtocol: inf.AppProtocolH2C,
		},
	}

	info := InferencePoolBuilder()(nil, pool)
	if info == nil || info.Service == nil {
		t.Fatalf("InferencePoolBuilder returned nil service info")
	}
	if len(info.Service.Ports) != len(pool.Spec.TargetPorts) {
		t.Fatalf("got %d ports, want %d", len(info.Service.Ports), len(pool.Spec.TargetPorts))
	}
	for _, port := range info.Service.Ports {
		if port.AppProtocol != workloadapi.AppProtocol_HTTP2 {
			t.Fatalf("port %d AppProtocol = %v, want %v", port.ServicePort, port.AppProtocol, workloadapi.AppProtocol_HTTP2)
		}
	}
}
