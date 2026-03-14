package jwks

import (
	"testing"

	"github.com/stretchr/testify/assert"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
)

func TestOwnersFromPolicyUseAttachmentScopedPaths(t *testing.T) {
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
	assert.Len(t, owners, 8)
	assert.Equal(t, "AgentgatewayPolicy/default/example#spec.targetRefs[0].traffic.jwtAuthentication.providers[1].jwks.remote", owners[0].ID.String())
	assert.Equal(t, "AgentgatewayPolicy/default/example#spec.targetRefs[0].backend.mcp.authentication.jwks", owners[1].ID.String())
	assert.Equal(t, "AgentgatewayPolicy/default/example#spec.targetRefs[3].traffic.jwtAuthentication.providers[1].jwks.remote", owners[6].ID.String())
	assert.Equal(t, "AgentgatewayPolicy/default/example#spec.targetRefs[3].backend.mcp.authentication.jwks", owners[7].ID.String())
}
