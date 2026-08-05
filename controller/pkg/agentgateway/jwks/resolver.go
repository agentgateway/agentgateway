package jwks

import (
	"fmt"
	"time"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type ResolvedJwksRequest struct {
	OwnerID JwksOwnerID
	Target  remotehttp.ResolvedTarget
	TTL     time.Duration
}

type Resolver interface {
	ResolveOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (*ResolvedJwksRequest, error)
}

// BackendRefGrantChecker reports whether a cross-namespace backendRef is
// permitted by a ReferenceGrant. It is the subset of
// plugins.ReferenceGrantChecker this package needs, declared locally because
// that package depends on this one. translator.ReferenceGrants satisfies it.
type BackendRefGrantChecker interface {
	BackendAllowed(
		ctx krt.HandlerContext,
		k schema.GroupVersionKind,
		backendName gwv1b1.ObjectName,
		backendNamespace gwv1b1.Namespace,
		sourceNamespace string,
		refKind schema.GroupKind,
		mode apisettings.BackendRefGrantMode,
	) bool
}

type defaultResolver struct {
	endpointResolver remotehttp.Resolver
	grants           BackendRefGrantChecker
	grantMode        apisettings.BackendRefGrantMode
}

// NewResolver builds a Resolver for remote JWKS owners. grants and grantMode
// govern cross-namespace backendRefs and are required rather than optional: a
// nil checker combined with a mode that demands grants rejects every
// cross-namespace reference, so callers must make the choice explicitly.
func NewResolver(
	endpointResolver remotehttp.Resolver,
	grants BackendRefGrantChecker,
	grantMode apisettings.BackendRefGrantMode,
) Resolver {
	return &defaultResolver{
		endpointResolver: endpointResolver,
		grants:           grants,
		grantMode:        grantMode,
	}
}

func (r *defaultResolver) ResolveOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
	if err := r.checkBackendRefGrant(krtctx, owner); err != nil {
		return nil, err
	}

	endpoint, err := ResolveEndpoint(krtctx, r.endpointResolver, owner.ID.Name, owner.DefaultNamespace, owner.Remote)
	if err != nil {
		return nil, err
	}

	return &ResolvedJwksRequest{
		OwnerID: owner.ID,
		Target:  *endpoint,
		TTL:     owner.TTL,
	}, nil
}

// checkBackendRefGrant mirrors the policy backendRef check in
// plugins.BuildBackendRef: the JWKS backendRef is a policy-level reference, so
// it is subject to the same BackendRefGrantMode setting.
func (r *defaultResolver) checkBackendRefGrant(krtctx krt.HandlerContext, owner RemoteJwksOwner) error {
	if !r.grantMode.RequirePolicyBackendGrant() {
		return nil
	}

	ref := owner.Remote.BackendRef
	if ref.Namespace == nil || string(*ref.Namespace) == owner.DefaultNamespace {
		return nil
	}

	sourceGVK := ownerSourceGVK(owner.ID.Kind)
	refGK := schema.GroupKind{
		Group: string(ptr.OrDefault(ref.Group, "")),
		Kind:  string(ptr.OrDefault(ref.Kind, wellknown.ServiceKind)),
	}
	if r.grants != nil && r.grants.BackendAllowed(
		krtctx,
		sourceGVK,
		ref.Name,
		*ref.Namespace,
		owner.DefaultNamespace,
		refGK,
		r.grantMode,
	) {
		return nil
	}

	return fmt.Errorf(
		"backendRef %v/%v not accessible to an %s in namespace %q (missing a ReferenceGrant?)",
		*ref.Namespace,
		ref.Name,
		sourceGVK.Kind,
		owner.DefaultNamespace,
	)
}

func ownerSourceGVK(kind OwnerKind) schema.GroupVersionKind {
	switch kind {
	case OwnerKindBackend:
		return wellknown.AgentgatewayBackendGVK
	default:
		return wellknown.AgentgatewayPolicyGVK
	}
}
