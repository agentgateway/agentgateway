package jwks

import (
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type ResolvedJwksRequest struct {
	OwnerID remotecache.OwnerID
	Target  remotehttp.ResolvedTarget
	TTL     time.Duration
}

// RequestKey derives the fetch key from the resolved target. A method rather
// than a field so the fetch-request collection and the translation-time
// lookup cannot diverge on key derivation (mirrors ResolvedOidcRequest).
func (r *ResolvedJwksRequest) RequestKey() remotehttp.FetchKey {
	return r.Target.Target.Key()
}

type Resolver interface {
	ResolveOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (*ResolvedJwksRequest, error)
}

type defaultResolver struct {
	endpointResolver remotehttp.Resolver
}

func NewResolver(endpointResolver remotehttp.Resolver) Resolver {
	return &defaultResolver{endpointResolver: endpointResolver}
}

func (r *defaultResolver) ResolveOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
	endpoint, err := resolveEndpoint(krtctx, r.endpointResolver, owner.ID.Name, owner.ID.Namespace, owner.Remote)
	if err != nil {
		return nil, err
	}

	return &ResolvedJwksRequest{
		OwnerID: owner.ID,
		Target:  *endpoint,
		TTL:     owner.TTL,
	}, nil
}

func resolveEndpoint(
	krtctx krt.HandlerContext,
	resolver remotehttp.Resolver,
	policyName, defaultNS string,
	remoteProvider agentgateway.RemoteJWKS,
) (*remotehttp.ResolvedTarget, error) {
	if resolver == nil {
		return nil, remotehttp.ErrResolverNotInitialized
	}

	return resolver.Resolve(krtctx, remotehttp.ResolveInput{
		ParentName:       policyName,
		DefaultNamespace: defaultNS,
		BackendRef:       remoteProvider.BackendRef,
		Path:             remoteProvider.JwksPath,
	})
}
