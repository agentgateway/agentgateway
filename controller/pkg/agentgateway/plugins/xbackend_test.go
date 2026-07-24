package plugins

import (
	"errors"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestResolveExternalHostnameXBackend(t *testing.T) {
	backend := &gwxv1a1.XBackend{
		ObjectMeta: metav1.ObjectMeta{Name: "external-api", Namespace: "default"},
		Spec: gwxv1a1.BackendSpec{
			Type: gwxv1a1.BackendTypeExternalHostname,
			Port: gwxv1a1.BackendPort{Port: 8443},
			ExternalHostname: &gwxv1a1.ExternalHostnameBackend{
				Hostname: "api.example.com",
			},
		},
	}
	backends := krt.NewStaticCollection(nil, []*gwxv1a1.XBackend{backend}, krt.WithName("plugins/TestResolveExternalHostnameXBackend"))
	agw := &AgwCollections{XBackends: backends}

	ref, err := DefaultRouteBackend(
		krt.TestingDummyContext{},
		agw,
		"default",
		wellknown.XBackendGVK.GroupKind(),
		"external-api",
		nil,
		new(gwv1.PortNumber(8443)),
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

func TestResolveExternalHostnameXBackendRejectsPortMismatch(t *testing.T) {
	backend := &gwxv1a1.XBackend{
		ObjectMeta: metav1.ObjectMeta{Name: "external-api", Namespace: "default"},
		Spec: gwxv1a1.BackendSpec{
			Type:             gwxv1a1.BackendTypeExternalHostname,
			Port:             gwxv1a1.BackendPort{Port: 443},
			ExternalHostname: &gwxv1a1.ExternalHostnameBackend{Hostname: "api.example.com"},
		},
	}
	agw := &AgwCollections{
		XBackends: krt.NewStaticCollection(nil, []*gwxv1a1.XBackend{backend}, krt.WithName("plugins/TestResolveExternalHostnameXBackendRejectsPortMismatch")),
	}

	_, err := DefaultRouteBackend(
		krt.TestingDummyContext{},
		agw,
		"default",
		wellknown.XBackendGVK.GroupKind(),
		"external-api",
		nil,
		new(gwv1.PortNumber(8443)),
	)
	var backendErr *BackendReferenceError
	if !errors.As(err, &backendErr) || backendErr.Reason != BackendReferenceErrorReasonUnsupportedValue {
		t.Fatalf("error = %v, want unsupported value", err)
	}
}
