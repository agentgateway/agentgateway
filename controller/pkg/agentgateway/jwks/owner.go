package jwks

import (
	"fmt"
	"reflect"
	"time"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

type OwnerKind string

const (
	OwnerKindPolicy  OwnerKind = "AgentgatewayPolicy"
	OwnerKindBackend OwnerKind = "AgentgatewayBackend"
)

type JwksOwnerID struct {
	Kind      OwnerKind
	Namespace string
	Name      string
	Path      string
}

func (o JwksOwnerID) String() string {
	return fmt.Sprintf("%s/%s/%s#%s", o.Kind, o.Namespace, o.Name, o.Path)
}

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
	var owners []RemoteJwksOwner

	for targetIdx := range policy.Spec.TargetRefs {
		if policy.Spec.Traffic != nil && policy.Spec.Traffic.JWTAuthentication != nil {
			for providerIdx, provider := range policy.Spec.Traffic.JWTAuthentication.Providers {
				if provider.JWKS.Remote == nil {
					continue
				}
				owners = append(owners, RemoteJwksOwner{
					ID: JwksOwnerID{
						Kind:      OwnerKindPolicy,
						Namespace: policy.Namespace,
						Name:      policy.Name,
						Path:      fmt.Sprintf("spec.targetRefs[%d].traffic.jwtAuthentication.providers[%d].jwks.remote", targetIdx, providerIdx),
					},
					DefaultNamespace: policy.Namespace,
					Remote:           *provider.JWKS.Remote,
					TTL:              ttlForRemote(*provider.JWKS.Remote),
				})
			}
		}

		if policy.Spec.Backend != nil && policy.Spec.Backend.MCP != nil && policy.Spec.Backend.MCP.Authentication != nil {
			owners = append(owners, RemoteJwksOwner{
				ID: JwksOwnerID{
					Kind:      OwnerKindPolicy,
					Namespace: policy.Namespace,
					Name:      policy.Name,
					Path:      fmt.Sprintf("spec.targetRefs[%d].backend.mcp.authentication.jwks", targetIdx),
				},
				DefaultNamespace: policy.Namespace,
				Remote:           policy.Spec.Backend.MCP.Authentication.JWKS,
				TTL:              ttlForRemote(policy.Spec.Backend.MCP.Authentication.JWKS),
			})
		}
	}

	return owners
}

func OwnersFromBackend(backend *agentgateway.AgentgatewayBackend) []RemoteJwksOwner {
	if backend.Spec.MCP == nil || backend.Spec.Policies == nil || backend.Spec.Policies.MCP == nil || backend.Spec.Policies.MCP.Authentication == nil {
		return nil
	}

	return []RemoteJwksOwner{{
		ID: JwksOwnerID{
			Kind:      OwnerKindBackend,
			Namespace: backend.Namespace,
			Name:      backend.Name,
			Path:      "spec.policies.mcp.authentication.jwks",
		},
		DefaultNamespace: backend.Namespace,
		Remote:           backend.Spec.Policies.MCP.Authentication.JWKS,
		TTL:              ttlForRemote(backend.Spec.Policies.MCP.Authentication.JWKS),
	}}
}

func PolicyJWTProviderLookupOwner(namespace, name string, providerIndex int, remote agentgateway.RemoteJWKS) RemoteJwksOwner {
	return RemoteJwksOwner{
		ID: JwksOwnerID{
			Kind:      OwnerKindPolicy,
			Namespace: namespace,
			Name:      name,
			Path:      fmt.Sprintf("spec.traffic.jwtAuthentication.providers[%d].jwks.remote", providerIndex),
		},
		DefaultNamespace: namespace,
		Remote:           remote,
		TTL:              ttlForRemote(remote),
	}
}

func PolicyBackendMCPAuthenticationLookupOwner(namespace, name string, remote agentgateway.RemoteJWKS) RemoteJwksOwner {
	return RemoteJwksOwner{
		ID: JwksOwnerID{
			Kind:      OwnerKindPolicy,
			Namespace: namespace,
			Name:      name,
			Path:      "spec.backend.mcp.authentication.jwks",
		},
		DefaultNamespace: namespace,
		Remote:           remote,
		TTL:              ttlForRemote(remote),
	}
}

func ttlForRemote(remote agentgateway.RemoteJWKS) time.Duration {
	if remote.CacheDuration == nil {
		return 5 * time.Minute
	}
	return remote.CacheDuration.Duration
}
