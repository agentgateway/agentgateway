package syncer

import (
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

type agentgatewaySyncerConfig struct {
	GatewayTransformationFunc   translator.GatewayTransformationFunction
	CustomResourceCollections   func(cfg CustomResourceCollectionsConfig)
	BuildAddressCollectionsFunc AgentgatewayAddressBuilderFunc
	BuildReferenceTypesFunc     func(agw *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes
}

type AgentgatewaySyncerOption func(*agentgatewaySyncerConfig)

func processAgentgatewaySyncerOptions(opts ...AgentgatewaySyncerOption) *agentgatewaySyncerConfig {
	cfg := &agentgatewaySyncerConfig{}
	for _, fn := range opts {
		fn(cfg)
	}
	return cfg
}

func WithGatewayTransformationFunc(f translator.GatewayTransformationFunction) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.GatewayTransformationFunc = f
		}
	}
}

func WithCustomResourceCollections(f func(cfg CustomResourceCollectionsConfig)) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.CustomResourceCollections = f
		}
	}
}

type AgentgatewayAddressBuilderFunc func(agw *plugins.AgwCollections, krtopts krtutil.KrtOptions) (krt.Collection[Address], func() bool)

// WithBuildAddressCollections provides a function to build the address collections for the syncer.
// This gives full control over how ServiceInfo and WorkloadInfo are constructed from the
// AgwCollections. The default implementation uses the istio ambient builder (see
// defaultBuildAddressCollections in syncer.go).
func WithBuildAddressCollections(f AgentgatewayAddressBuilderFunc) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.BuildAddressCollectionsFunc = f
		}
	}
}

func WithBuildReferenceTypes(f func(agw *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.BuildReferenceTypesFunc = f
		}
	}
}
