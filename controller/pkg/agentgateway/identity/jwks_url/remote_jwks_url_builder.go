package jwks_url

import (
	"crypto/tls"
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/backendtransport"
)

type JwksUrlBuilder interface {
	BuildJwksUrlAndTlsConfig(krtctx krt.HandlerContext, policyName, defaultNS string, remoteProvider *agentgateway.RemoteJWKS) (string, *tls.Config, error)
}

type defaultJwksUrlFactory struct {
	lookup *backendtransport.BackendTransportLookup
}

func NewJwksUrlFactory(lookup *backendtransport.BackendTransportLookup) JwksUrlBuilder {
	return &defaultJwksUrlFactory{
		lookup: lookup,
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
