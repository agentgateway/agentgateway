package deployer

import (
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/collections"
)

// Inputs is the set of options used to configure gateway/inference pool deployment.
type Inputs struct {
	Dev                        bool
	ControlPlane               ControlPlaneInfo
	CommonCollections          *collections.CommonCollections
	AgentgatewayClassName      string
	AgentgatewayControllerName string
}

// InMemoryGatewayParametersConfig holds the configuration for creating in-memory GatewayParameters.
type InMemoryGatewayParametersConfig struct {
	ControllerName             string
	ClassName                  string
	ImageInfo                  *ImageInfo
	WaypointClassName          string
	AgwControllerName          string
	OmitDefaultSecurityContext bool
}
