package plugins

import (
	"testing"

	"istio.io/istio/pkg/kube/krt"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/agentgateway/agentgateway/api"
	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestBackendKindsUseDistinctKeys(t *testing.T) {
	backend := &gwxv1a1.XBackend{
		Name: "shared", Namespace: "default",
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
		Backends: krt.NewStaticCollection(nil, []*agentgateway.AgentgatewayBackend{{
			Name: "shared", Namespace: "default",
		}}, krt.WithName("plugins/TestBackendKindsUseDistinctKeys")),
	}

	xBackendRef, err := DefaultRouteBackend(
		krt.TestingDummyContext{},
		agw,
		"default",
		wellknown.XBackendGVK.GroupKind(),
		"shared",
		nil,
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	resolvedXBackend, ok := xBackendRef.Kind.(*api.BackendReference_Backend)
	if !ok {
		t.Fatalf("XBackend reference kind = %T, want backend", xBackendRef.Kind)
	}
	if resolvedXBackend.Backend != "gateway.networking.x-k8s.io/XBackend/default/shared" {
		t.Fatalf("unexpected XBackend reference: %+v", xBackendRef)
	}

	agwBackendRef, err := DefaultRouteBackend(
		krt.TestingDummyContext{},
		agw,
		"default",
		wellknown.AgentgatewayBackendGVK.GroupKind(),
		"shared",
		nil,
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	resolvedAgwBackend, ok := agwBackendRef.Kind.(*api.BackendReference_Backend)
	if !ok {
		t.Fatalf("AgentgatewayBackend reference kind = %T, want backend", agwBackendRef.Kind)
	}
	if resolvedAgwBackend.Backend != "default/shared" {
		t.Fatalf("unexpected AgentgatewayBackend reference: %+v", agwBackendRef)
	}
	if resolvedXBackend.Backend == resolvedAgwBackend.Backend {
		t.Fatalf("backend references collide at %q", resolvedXBackend.Backend)
	}
}
