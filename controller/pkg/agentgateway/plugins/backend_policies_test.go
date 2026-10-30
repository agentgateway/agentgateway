package plugins

import (
	"testing"

	"istio.io/istio/pkg/test/util/assert"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func backendTLSPolicy(tls *agentgateway.BackendTLS) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		Namespace: "default", Name: "backend-tls",
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{
				TLS: tls,
			},
		},
	}
}

func TestTranslateBackendTLSMtlsCertificateFile(t *testing.T) {
	ctx := PolicyCtx{}
	policy := backendTLSPolicy(&agentgateway.BackendTLS{
		MtlsCertificateRef: []agentgateway.BackendTLSCertificateRef{{
			Kind: "File",
			File: &agentgateway.BackendTLSFile{
				Cert: "/etc/agentgateway/certs/tls.crt",
				Key:  "/etc/agentgateway/certs/tls.key",
				Root: "/etc/agentgateway/certs/ca.crt",
			},
		}},
	})

	got, err := translateBackendTLS(ctx, policy)
	assert.NoError(t, err)

	btls := got.GetBackend().GetBackendTls()
	assert.Equal(t, btls.GetCertPath(), "/etc/agentgateway/certs/tls.crt")
	assert.Equal(t, btls.GetKeyPath(), "/etc/agentgateway/certs/tls.key")
	assert.Equal(t, btls.GetRootPath(), "/etc/agentgateway/certs/ca.crt")
	// The controller only forwards paths; it must not read the files.
	assert.Equal(t, len(btls.GetCert()), 0)
	assert.Equal(t, len(btls.GetKey()), 0)
}

func TestTranslateBackendTLSMtlsCertificateFileWithoutRoot(t *testing.T) {
	ctx := PolicyCtx{}
	policy := backendTLSPolicy(&agentgateway.BackendTLS{
		MtlsCertificateRef: []agentgateway.BackendTLSCertificateRef{{
			Kind: "File",
			File: &agentgateway.BackendTLSFile{
				Cert: "/etc/agentgateway/certs/tls.crt",
				Key:  "/etc/agentgateway/certs/tls.key",
			},
		}},
	})

	got, err := translateBackendTLS(ctx, policy)
	assert.NoError(t, err)

	btls := got.GetBackend().GetBackendTls()
	assert.Equal(t, btls.GetCertPath(), "/etc/agentgateway/certs/tls.crt")
	assert.Equal(t, btls.GetKeyPath(), "/etc/agentgateway/certs/tls.key")
	assert.Equal(t, btls.GetRootPath(), "")
}
