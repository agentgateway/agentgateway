package plugins

import (
	"maps"

	"k8s.io/apimachinery/pkg/runtime/schema"
)

type AgwPlugin struct {
	AddResourceExtension *AddResourcesPlugin
	ContributesPolicies  map[schema.GroupKind]PolicyPlugin
}

func MergePlugins(plug ...AgwPlugin) AgwPlugin {
	ret := AgwPlugin{
		ContributesPolicies: make(map[schema.GroupKind]PolicyPlugin),
	}
	for _, p := range plug {
		// Merge contributed policies
		maps.Copy(ret.ContributesPolicies, p.ContributesPolicies)
		if p.AddResourceExtension != nil {
			if ret.AddResourceExtension == nil {
				ret.AddResourceExtension = &AddResourcesPlugin{}
			}
			if ret.AddResourceExtension.Binds == nil {
				ret.AddResourceExtension.Binds = p.AddResourceExtension.Binds
			}
			if p.AddResourceExtension.Listeners != nil {
				ret.AddResourceExtension.Listeners = p.AddResourceExtension.Listeners
			}
			if p.AddResourceExtension.Routes != nil {
				ret.AddResourceExtension.Routes = p.AddResourceExtension.Routes
			}
		}
	}
	return ret
}

// Plugins registers all built-in policy plugins
func Plugins(agw *AgwCollections) []AgwPlugin {
	return []AgwPlugin{
		NewAgentPlugin(agw),
		NewInferencePlugin(agw),
		NewA2APlugin(agw),
		NewBackendTLSPlugin(agw),
	}
}
