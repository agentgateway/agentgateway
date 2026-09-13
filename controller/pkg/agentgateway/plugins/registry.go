package plugins

import (
	"maps"
	"reflect"
	"slices"
	"strings"

	"k8s.io/apimachinery/pkg/runtime/schema"
)

type AgwPlugin struct {
	AddResourceExtension *AddResourcesPlugin
	ContributesPolicies  map[schema.GroupKind]PolicyPlugin
	ContributesBackends  map[schema.GroupKind]BackendPlugin
}

// MergePlugins combines the contributions of every plugin into a single AgwPlugin.
// AddResourcesPlugin collection fields must be disjoint across the set; compose two
// plugins yourself if one adds to the other's contribution.
func MergePlugins(plug ...AgwPlugin) AgwPlugin {
	ret := AgwPlugin{
		ContributesPolicies: make(map[schema.GroupKind]PolicyPlugin),
		ContributesBackends: make(map[schema.GroupKind]BackendPlugin),
	}
	for _, p := range plug {
		// Merge contributed policies
		maps.Copy(ret.ContributesPolicies, p.ContributesPolicies)
		maps.Copy(ret.ContributesBackends, p.ContributesBackends)
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
			if p.AddResourceExtension.AncestorBackends != nil {
				ret.AddResourceExtension.AncestorBackends = p.AddResourceExtension.AncestorBackends
			}
			if p.AddResourceExtension.GatewayStatuses != nil {
				ret.AddResourceExtension.GatewayStatuses = p.AddResourceExtension.GatewayStatuses
			}
			for _, r := range p.AddResourceExtension.ParentResolvers {
				if r != nil {
					ret.AddResourceExtension.ParentResolvers = append(ret.AddResourceExtension.ParentResolvers, r)
				}
			}
		}
	}
	if contested := contestedResourceExtensionFields(plug); len(contested) > 0 {
		logger.Error("addResourceExtension fields contributed by more than one plugin, all but one contribution is discarded",
			"fields", strings.Join(contested, ", "))
	}
	return ret
}

// contestedResourceExtensionFields names the collection fields more than one plugin
// populates. Collection fields are interfaces; ParentResolvers is a slice and
// accumulates across plugins by design.
func contestedResourceExtensionFields(plug []AgwPlugin) []string {
	contributors := map[string]int{}
	for _, p := range plug {
		if p.AddResourceExtension == nil {
			continue
		}
		v := reflect.ValueOf(*p.AddResourceExtension)
		for i := range v.NumField() {
			if f := v.Field(i); f.Kind() == reflect.Interface && !f.IsNil() {
				contributors[v.Type().Field(i).Name]++
			}
		}
	}
	var contested []string
	for name, n := range contributors {
		if n > 1 {
			contested = append(contested, name)
		}
	}
	slices.Sort(contested)
	return contested
}
