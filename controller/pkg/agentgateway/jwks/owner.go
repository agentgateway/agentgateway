package jwks

import (
	"fmt"
	"reflect"
	"time"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
)

// JwksOwnerID is the JWKS-flavored alias for remotecache.OwnerID; identical
// layout, package-local name for readability at JWKS call sites.
type JwksOwnerID = remotecache.OwnerID

type OwnerKey = JwksOwnerID

type RemoteJwksOwner struct {
	ID               JwksOwnerID
	DefaultNamespace string
	Remote           agentgateway.RemoteJWKS
	TTL              time.Duration
}

func (o RemoteJwksOwner) ResourceName() string {
	return o.ID.String()
}

func (o RemoteJwksOwner) Equals(other RemoteJwksOwner) bool {
	return o.ID == other.ID &&
		o.DefaultNamespace == other.DefaultNamespace &&
		o.TTL == other.TTL &&
		reflect.DeepEqual(o.Remote, other.Remote)
}

func OwnersFromPolicy(policy *agentgateway.AgentgatewayPolicy) []RemoteJwksOwner {
	if len(policy.Spec.TargetRefs) == 0 {
		return nil
	}

	var owners []RemoteJwksOwner

	if policy.Spec.Traffic != nil && policy.Spec.Traffic.JWTAuthentication != nil {
		for providerIdx, provider := range policy.Spec.Traffic.JWTAuthentication.Providers {
			owner, ok := PolicyJWTProviderLookupOwner(policy.Namespace, policy.Name, providerIdx, provider)
			if !ok {
				continue
			}
			owners = append(owners, owner)
		}
	}

	if policy.Spec.Backend != nil && policy.Spec.Backend.MCP != nil && policy.Spec.Backend.MCP.Authentication != nil {
		owners = append(owners, PolicyBackendMCPAuthenticationLookupOwner(
			policy.Namespace,
			policy.Name,
			policy.Spec.Backend.MCP.Authentication.JWKS,
		))
	}

	return owners
}

func OwnersFromBackend(backend *agentgateway.AgentgatewayBackend) []RemoteJwksOwner {
	if backend.Spec.MCP == nil || backend.Spec.Policies == nil || backend.Spec.Policies.MCP == nil || backend.Spec.Policies.MCP.Authentication == nil {
		return nil
	}

	return []RemoteJwksOwner{backendMCPAuthenticationOwner(
		backend.Namespace,
		backend.Name,
		backend.Spec.Policies.MCP.Authentication.JWKS,
	)}
}

func PolicyJWTProviderLookupOwner(namespace, name string, providerIndex int, provider agentgateway.JWTProvider) (RemoteJwksOwner, bool) {
	if provider.JWKS.Remote == nil {
		return RemoteJwksOwner{}, false
	}

	return RemoteJwksOwner{
		ID: JwksOwnerID{
			Kind:      remotecache.OwnerKindPolicy,
			Namespace: namespace,
			Name:      name,
			Path:      fmt.Sprintf("spec.traffic.jwtAuthentication.providers[%d].jwks.remote", providerIndex),
		},
		DefaultNamespace: namespace,
		Remote:           *provider.JWKS.Remote.DeepCopy(),
		TTL:              TTLForRemote(*provider.JWKS.Remote),
	}, true
}

func PolicyBackendMCPAuthenticationLookupOwner(namespace, name string, remote agentgateway.RemoteJWKS) RemoteJwksOwner {
	return RemoteJwksOwner{
		ID: JwksOwnerID{
			Kind:      remotecache.OwnerKindPolicy,
			Namespace: namespace,
			Name:      name,
			Path:      "spec.backend.mcp.authentication.jwks",
		},
		DefaultNamespace: namespace,
		Remote:           *remote.DeepCopy(),
		TTL:              TTLForRemote(remote),
	}
}

func backendMCPAuthenticationOwner(namespace, name string, remote agentgateway.RemoteJWKS) RemoteJwksOwner {
	return RemoteJwksOwner{
		ID: JwksOwnerID{
			Kind:      remotecache.OwnerKindBackend,
			Namespace: namespace,
			Name:      name,
			Path:      "spec.policies.mcp.authentication.jwks",
		},
		DefaultNamespace: namespace,
		Remote:           *remote.DeepCopy(),
		TTL:              TTLForRemote(remote),
	}
}

func TTLForRemote(remote agentgateway.RemoteJWKS) time.Duration {
	if remote.CacheDuration == nil {
		return 5 * time.Minute
	}
	return remote.CacheDuration.Duration
}
