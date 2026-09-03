package plugins

import (
	"testing"

	"istio.io/istio/pkg/kube/krt"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/agentgateway/agentgateway/api"
	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestResolveExternalHostnameXBackend(t *testing.T) {
	backend := &gwxv1a1.XBackend{
		Name: "external-api", Namespace: "default",
		Spec: gwxv1a1.BackendSpec{
			Type: gwxv1a1.BackendTypeExternalHostname,
			Port: gwxv1a1.BackendPort{Port: 8443},
			ExternalHostname: &gwxv1a1.ExternalHostnameBackend{
				Hostname: "api.example.com",
			},
		},
	}
	backends := krt.NewStaticCollection(nil, []*gwxv1a1.XBackend{backend}, krt.WithName("plugins/TestResolveExternalHostnameXBackend"))
	agw := &AgwCollections{
		Settings:  apisettings.Settings{EnableXBackend: true},
		XBackends: backends,
	}

	ref, err := DefaultRouteBackend(
		krt.TestingDummyContext{},
		agw,
		"default",
		wellknown.XBackendGVK.GroupKind(),
		"external-api",
		nil,
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	resolved, ok := ref.Kind.(*api.BackendReference_Backend)
	if !ok {
		t.Fatalf("backend kind = %T, want backend", ref.Kind)
	}
	if resolved.Backend != "default/external-api" {
		t.Fatalf("unexpected backend reference: %+v", ref)
	}
}
