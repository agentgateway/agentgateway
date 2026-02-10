package deployer

// TODO(tim): Consolidate with the other wellknown packages?
const (
	// AgentgatewayImage is the agentgateway image repository
	AgentgatewayImage = "agentgateway"
	// AgentgatewayRegistry is the agentgateway registry
	AgentgatewayRegistry = "cr.agentgateway.dev"
	// AgentgatewayDefaultTag is the default agentgateway image tag
	// Note: should be in sync with version in go.mod and test/deployer/testdata/*
	AgentgatewayDefaultTag = "0.11.2"
)
