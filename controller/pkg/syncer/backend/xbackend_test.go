package agentgatewaybackend

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
)

func TestBuildXBackendHTTP2WithServerTLS(t *testing.T) {
	protocol := gwxv1a1.BackendProtocolHTTP2
	backend := externalXBackend("api", 443)
	backend.Spec.Protocol = &protocol
	backend.Spec.TLS = &gwxv1a1.BackendTLS{
		Mode: gwxv1a1.BackendTLSModeServerOnly,
		Validation: gwv1.BackendTLSPolicyValidation{
			WellKnownCACertificates: new(gwv1.WellKnownCACertificatesSystem),
			Hostname:                "api.example.com",
			SubjectAltNames: []gwv1.SubjectAltName{{
				Type:     gwv1.HostnameSubjectAltNameType,
				Hostname: "backend.example.com",
			}},
		},
	}

	got, err := buildXBackend(krt.TestingDummyContext{}, &plugins.AgwCollections{}, backend, nil)
	if err != nil {
		t.Fatal(err)
	}
	if got.GetStatic().Host != "api.example.com" || got.GetStatic().Port != 443 {
		t.Fatalf("static backend = %+v", got.GetStatic())
	}
	if len(got.InlinePolicies) != 2 {
		t.Fatalf("inline policies = %d, want 2", len(got.InlinePolicies))
	}
	if http := got.InlinePolicies[0].GetBackendHttp(); http == nil || http.Version != api.BackendPolicySpec_BackendHTTP_HTTP2 {
		t.Fatalf("HTTP policy = %+v", http)
	}
	tls := got.InlinePolicies[1].GetBackendTls()
	if tls == nil || tls.GetHostname() != "api.example.com" || len(tls.VerifySubjectAltNames) != 1 || tls.VerifySubjectAltNames[0] != "backend.example.com" {
		t.Fatalf("TLS policy = %+v", tls)
	}
}

func TestBuildXBackendProtocolValidation(t *testing.T) {
	tests := []struct {
		name     string
		protocol gwxv1a1.BackendProtocol
		tls      *gwxv1a1.BackendTLS
		want     string
	}{
		{
			name:     "h2c with tls",
			protocol: gwxv1a1.BackendProtocolH2C,
			tls: &gwxv1a1.BackendTLS{
				Mode:       gwxv1a1.BackendTLSModeServerOnly,
				Validation: gwv1.BackendTLSPolicyValidation{WellKnownCACertificates: new(gwv1.WellKnownCACertificatesSystem), Hostname: "api.example.com"},
			},
			want: "H2C cannot be combined with TLS",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			backend := externalXBackend("api", 443)
			backend.Spec.Protocol = &tt.protocol
			backend.Spec.TLS = tt.tls
			_, err := buildXBackend(krt.TestingDummyContext{}, &plugins.AgwCollections{}, backend, nil)
			if err == nil || !strings.Contains(err.Error(), tt.want) {
				t.Fatalf("error = %v, want %q", err, tt.want)
			}
		})
	}
}

func TestBuildXBackendMCPUsesStreamableHTTPDefaults(t *testing.T) {
	protocol := gwxv1a1.BackendProtocolMCP
	backend := externalXBackend("mcp-api", 443)
	backend.Spec.Protocol = &protocol
	backend.Spec.TLS = &gwxv1a1.BackendTLS{
		Mode: gwxv1a1.BackendTLSModeServerOnly,
		Validation: gwv1.BackendTLSPolicyValidation{
			WellKnownCACertificates: new(gwv1.WellKnownCACertificatesSystem),
			Hostname:                "api.example.com",
		},
	}

	got, err := buildXBackend(krt.TestingDummyContext{}, &plugins.AgwCollections{}, backend, nil)
	if err != nil {
		t.Fatal(err)
	}
	mcp := got.GetMcp()
	if mcp == nil || len(mcp.Targets) != 1 {
		t.Fatalf("MCP backend = %+v", mcp)
	}
	target := mcp.Targets[0]
	service := target.Backend.GetService()
	if target.Name != "default" || target.Protocol != api.MCPTarget_STREAMABLE_HTTP || target.Path != "" {
		t.Fatalf("MCP target defaults = %+v", target)
	}
	if service == nil || service.Hostname != "api.example.com" || service.Namespace != "default" || target.Backend.Port != 443 {
		t.Fatalf("MCP target backend = %+v", target.Backend)
	}
	if mcp.StatefulMode != api.MCPBackend_STATEFUL || mcp.PrefixMode != api.MCPBackend_CONDITIONAL || mcp.FailureMode != api.MCPBackend_FAIL_CLOSED {
		t.Fatalf("MCP behavior defaults = %+v", mcp)
	}
	if len(got.InlinePolicies) != 1 || got.InlinePolicies[0].GetBackendTls() == nil {
		t.Fatalf("inline policies = %+v", got.InlinePolicies)
	}
}

func TestBuildXBackendCrossNamespaceClientCertificateRequiresGrant(t *testing.T) {
	backend := externalXBackend("api", 443)
	backend.Spec.TLS = &gwxv1a1.BackendTLS{
		Mode: gwxv1a1.BackendTLSModeClientAndServer,
		ClientCertificateRef: &gwv1.SecretObjectReference{
			Name:      "client-cert",
			Namespace: new(gwv1.Namespace("certs")),
		},
		Validation: gwv1.BackendTLSPolicyValidation{
			WellKnownCACertificates: new(gwv1.WellKnownCACertificatesSystem),
			Hostname:                "api.example.com",
		},
	}

	_, err := buildXBackend(krt.TestingDummyContext{}, &plugins.AgwCollections{}, backend, nil)
	if err == nil || !strings.Contains(err.Error(), "not permitted by a ReferenceGrant") {
		t.Fatalf("error = %v", err)
	}
}

func TestBuildXBackendStatusIsPerGateway(t *testing.T) {
	backend := externalXBackend("api", 443)
	backend.Generation = 3
	status := buildXBackendStatus(backend, "agentgateway.dev/agentgateway", []types.NamespacedName{
		{Namespace: "default", Name: "gateway"},
	}, nil)
	if len(status.Ancestors) != 1 {
		t.Fatalf("ancestors = %d, want 1", len(status.Ancestors))
	}
	ancestor := status.Ancestors[0]
	if ancestor.AncestorRef.Name != "gateway" || len(ancestor.Conditions) != 1 || ancestor.Conditions[0].Status != metav1.ConditionTrue || ancestor.Conditions[0].ObservedGeneration != 3 {
		t.Fatalf("unexpected ancestor status: %+v", ancestor)
	}
}

func externalXBackend(name string, port gwxv1a1.PortNumber) *gwxv1a1.XBackend {
	return &gwxv1a1.XBackend{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: "default"},
		Spec: gwxv1a1.BackendSpec{
			Type:             gwxv1a1.BackendTypeExternalHostname,
			Port:             gwxv1a1.BackendPort{Port: port},
			ExternalHostname: &gwxv1a1.ExternalHostnameBackend{Hostname: "api.example.com"},
		},
	}
}
