package jwks

import (
	"testing"

	"github.com/stretchr/testify/assert"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
)

func TestOwnersFromPolicyUseCanonicalSpecScopedPaths(t *testing.T) {
	policy := &agentgateway.AgentgatewayPolicy{}
	policy.Namespace = "default"
	policy.Name = "example"
	policy.Spec.TargetRefs = make([]shared.LocalPolicyTargetReferenceWithSectionName, 4)
	policy.Spec.Traffic = &agentgateway.Traffic{
		JWTAuthentication: &agentgateway.JWTAuthentication{
			Providers: []agentgateway.JWTProvider{
				{},
				{
					JWKS: agentgateway.JWKS{Remote: &agentgateway.RemoteJWKS{}},
				},
			},
		},
	}
	policy.Spec.Backend = &agentgateway.BackendFull{
		MCP: &agentgateway.BackendMCP{
			Authentication: &agentgateway.MCPAuthentication{
				JWKS: agentgateway.RemoteJWKS{},
			},
		},
	}

	owners := OwnersFromPolicy(policy)
	assert.Len(t, owners, 2)
	assert.Equal(t, "AgentgatewayPolicy/default/example#spec.traffic.jwtAuthentication.providers[1].jwks.remote", owners[0].ID.String())
	assert.Equal(t, "AgentgatewayPolicy/default/example#spec.backend.mcp.authentication.jwks", owners[1].ID.String())
	owner, ok := PolicyJWTProviderLookupOwner(policy.Namespace, policy.Name, 1, policy.Spec.Traffic.JWTAuthentication.Providers[1])
	assert.True(t, ok)
	assert.Equal(t, owner, owners[0])
	assert.Equal(t, PolicyBackendMCPAuthenticationLookupOwner(policy.Namespace, policy.Name, policy.Spec.Backend.MCP.Authentication.JWKS), owners[1])
}

func TestOwnersFromPolicyRequireAtLeastOneTargetRef(t *testing.T) {
	policy := &agentgateway.AgentgatewayPolicy{}
	policy.Namespace = "default"
	policy.Name = "example"
	policy.Spec.Traffic = &agentgateway.Traffic{
		JWTAuthentication: &agentgateway.JWTAuthentication{
			Providers: []agentgateway.JWTProvider{{
				JWKS: agentgateway.JWKS{Remote: &agentgateway.RemoteJWKS{}},
			}},
		},
	}

	assert.Nil(t, OwnersFromPolicy(policy))
}
