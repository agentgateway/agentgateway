package controller

import (
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
)

func BuiltinAgwPlugins(agw *agwplugins.AgwCollections, jwksLookup jwks.Lookup) []agwplugins.AgwPlugin {
	return []agwplugins.AgwPlugin{
		agwplugins.NewAgentPlugin(agw, jwksLookup),
		agwplugins.NewInferencePlugin(agw),
		agwplugins.NewA2APlugin(agw),
		agwplugins.NewBackendTLSPlugin(agw),
	}
}
