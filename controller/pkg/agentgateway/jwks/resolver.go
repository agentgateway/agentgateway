package jwks

import (
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type ResolvedJwksRequest struct {
	OwnerID  JwksOwnerID
	Endpoint remotehttp.ResolvedEndpoint
	TTL      time.Duration
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
	endpoint, err := ResolveEndpoint(krtctx, r.endpointResolver, owner.ID.Name, owner.DefaultNamespace, &owner.Remote)
	if err != nil {
		return nil, err
	}

	return &ResolvedJwksRequest{
		OwnerID:  owner.ID,
		Endpoint: *endpoint,
		TTL:      owner.TTL,
	}, nil
}
