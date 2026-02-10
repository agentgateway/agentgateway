package deployer

import (
	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
)

// helmConfig stores the top-level helm values used by the deployer.
type HelmConfig struct {
	Agentgateway       *AgentgatewayHelmGateway `json:"agentgateway,omitempty"`
	InferenceExtension *HelmInferenceExtension  `json:"inferenceExtension,omitempty"`
}

// helmPort represents a Gateway Listener port
type HelmPort struct {
	Port       *int32  `json:"port,omitempty"`
	Protocol   *string `json:"protocol,omitempty"`
	Name       *string `json:"name,omitempty"`
	TargetPort *int32  `json:"targetPort,omitempty"`
	NodePort   *int32  `json:"nodePort,omitempty"`
}

// helmXds represents the xds host and port to which envoy will connect
// to receive xds config updates
type HelmXds struct {
	Host *string     `json:"host,omitempty"`
	Port *uint32     `json:"port,omitempty"`
	Tls  *HelmXdsTls `json:"tls,omitempty"`
}

type HelmXdsTls struct {
	Enabled *bool   `json:"enabled,omitempty"`
	CaCert  *string `json:"caCert,omitempty"`
}

type HelmInferenceExtension struct {
	EndpointPicker *HelmEndpointPickerExtension `json:"endpointPicker,omitempty"`
}

type HelmEndpointPickerExtension struct {
	PoolName      string `json:"poolName"`
	PoolNamespace string `json:"poolNamespace"`
}

type AgentgatewayHelmService struct {
	LoadBalancerIP *string `json:"loadBalancerIP,omitempty"`
}

type AgentgatewayHelmGateway struct {
	agentgateway.AgentgatewayParametersConfigs `json:",inline"`
	// naming
	Name               *string           `json:"name,omitempty"`
	GatewayClassName   *string           `json:"gatewayClassName,omitempty"`
	GatewayAnnotations map[string]string `json:"gatewayAnnotations,omitempty"`
	GatewayLabels      map[string]string `json:"gatewayLabels,omitempty"`

	// deployment/service values
	Ports   []HelmPort               `json:"ports,omitempty"`
	Service *AgentgatewayHelmService `json:"service,omitempty"`

	// agentgateway xds values
	Xds *HelmXds `json:"xds,omitempty"`
}
