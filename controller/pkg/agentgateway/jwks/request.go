package jwks

import (
	"crypto/tls"
	"errors"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

var (
	errResolverNotInitialized       = errors.New("remote http resolver hasn't been initialized")
	errRemoteProviderNotInitialized = errors.New("remote jwks provider hasn't been initialized")
)

func ResolveEndpoint(
	krtctx krt.HandlerContext,
	resolver remotehttp.Resolver,
	policyName, defaultNS string,
	remoteProvider *agentgateway.RemoteJWKS,
) (*remotehttp.ResolvedEndpoint, error) {
	if resolver == nil {
		return nil, errResolverNotInitialized
	}
	if remoteProvider == nil {
		return nil, errRemoteProviderNotInitialized
	}

	return resolver.Resolve(krtctx, remotehttp.ResolveInput{
		ParentName:       policyName,
		DefaultNamespace: defaultNS,
		BackendRef:       remoteProvider.BackendRef,
		Path:             remoteProvider.JwksPath,
	})
}

func BuildRequest(
	krtctx krt.HandlerContext,
	resolver remotehttp.Resolver,
	policyName, defaultNS string,
	remoteProvider *agentgateway.RemoteJWKS,
) (Request, *tls.Config, error) {
	endpoint, err := ResolveEndpoint(krtctx, resolver, policyName, defaultNS, remoteProvider)
	if err != nil {
		return Request{}, nil, err
	}
	return endpoint.Request, endpoint.TLSConfig, nil
}
