package jwks_url

import (
	"crypto/tls"
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/backendtransport"
)

type JwksUrlBuilder interface {
	BuildJwksUrlAndTlsConfig(krtctx krt.HandlerContext, policyName, defaultNS string, remoteProvider *agentgateway.RemoteJWKS) (string, *tls.Config, error)
}

var JwksUrlBuilderFactory = func() JwksUrlBuilder { return &emptyJwksUrlFactory{} }

type emptyJwksUrlFactory struct{}

func (f *emptyJwksUrlFactory) BuildJwksUrlAndTlsConfig(_ krt.HandlerContext, _, _ string, _ *agentgateway.RemoteJWKS) (string, *tls.Config, error) {
	return "", nil, fmt.Errorf("JwksUrlBuilderFactory must be initialized before use")
}

type defaultJwksUrlFactory struct {
	lookup *backendtransport.BackendTransportLookup
}

func NewJwksUrlFactory(cfgmaps krt.Collection[*corev1.ConfigMap],
	services krt.Collection[*corev1.Service],
	backends krt.Collection[*agentgateway.AgentgatewayBackend],
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy],
	backendTLSPolicies krt.Collection[*gwv1.BackendTLSPolicy],
) JwksUrlBuilder {
	return &defaultJwksUrlFactory{
		lookup: backendtransport.NewBackendTransportLookup(cfgmaps, services, backends, agentgatewayPolicies, backendTLSPolicies),
	}
}

func (f *defaultJwksUrlFactory) BuildJwksUrlAndTlsConfig(krtctx krt.HandlerContext, policyName, defaultNS string, remoteProvider *agentgateway.RemoteJWKS) (string, *tls.Config, error) {
	ref := remoteProvider.BackendRef
	path := strings.TrimPrefix(remoteProvider.JwksPath, "/")
	transport, err := f.lookup.Resolve(krtctx, policyName, defaultNS, ref, "")
	if err != nil {
		return "", nil, err
	}

	scheme := "http"
	if transport.TLSConfig != nil {
		scheme = "https"
	}
	return fmt.Sprintf("%s://%s/%s", scheme, transport.ConnectHost, path), transport.TLSConfig, nil
}
