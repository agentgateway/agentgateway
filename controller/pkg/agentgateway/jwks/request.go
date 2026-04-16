package jwks

import (
	"errors"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

var (
	errResolverNotInitialized = errors.New("remote http resolver hasn't been initialized")
)

func ResolveEndpoint(
	krtctx krt.HandlerContext,
	resolver remotehttp.Resolver,
	policyName, defaultNS string,
	remoteProvider agentgateway.RemoteJWKS,
) (*remotehttp.ResolvedTarget, error) {
	if resolver == nil {
		return nil, errResolverNotInitialized
	}

	return resolver.Resolve(krtctx, remotehttp.ResolveInput{
		ParentName:       policyName,
		DefaultNamespace: defaultNS,
		BackendRef:       remoteProvider.BackendRef,
		Path:             remoteProvider.JwksPath,
	})
}
