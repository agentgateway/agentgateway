package agentgatewaysyncer

import (
	"istio.io/istio/pilot/pkg/model"
	"istio.io/istio/pkg/workloadapi"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/translator"
)

type agentgatewaySyncerConfig struct {
	GatewayTransformationFunc   translator.GatewayTransformationFunction
	CustomResourceCollections   func(cfg CustomResourceCollectionsConfig)
	WorkloadAddressProviderFunc func(model.WorkloadInfo) *workloadapi.Address
	ServiceAddressProviderFunc  func(model.ServiceInfo) *workloadapi.Address
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

// WithWorkloadAddressProviderFunc provides a function to compute the Address for WorkloadInfo objects
// when AsAddress is not populated. This function is called during buildAddressCollections, after WorkloadInfo
// objects are created from Kubernetes resources (Pods, WorkloadEntries, etc.) by Istio's ambient workload builder,
// but before they are wrapped in Address structs and used for XDS generation. If the function returns a non-nil
// Address, it will be used to populate AsAddress (similar to PrecomputeWorkload).
// This allows downstream implementations to extend the address computation for workloads.
func WithWorkloadAddressProviderFunc(f func(model.WorkloadInfo) *workloadapi.Address) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.WorkloadAddressProviderFunc = f
		}
	}
}

// WithServiceAddressProviderFunc provides a function to compute the Address for ServiceInfo objects
// when AsAddress is not populated. This function is called during buildAddressCollections, after ServiceInfo
// objects are created from Kubernetes resources (Services, ServiceEntries, etc.) by Istio's ambient service builder,
// but before they are wrapped in Address structs and used for XDS generation. If the function returns a non-nil
// Address, it will be used to populate AsAddress (similar to precomputeService).
// This allows downstream implementations to extend the address computation for services.
func WithServiceAddressProviderFunc(f func(model.ServiceInfo) *workloadapi.Address) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.ServiceAddressProviderFunc = f
		}
	}
}
