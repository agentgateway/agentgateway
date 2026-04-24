package plugins

import (
	"sigs.k8s.io/gateway-api/pkg/features"
)

const SupportOIDC features.FeatureName = "OIDC"

// AgentgatewaySupportedFeatures lists the features advertised by this
// controller beyond Gateway API's `features.AllFeatures`. The deployer
// composes these into the GatewayClass `status.supportedFeatures`; see
// `deployer.GetSupportedFeaturesForAgentGateway`.
//
// Composing statically (mirroring Gateway API's own `AllFeatures`
// composition) avoids mutating library-owned global state from `init()`.
var AgentgatewaySupportedFeatures = []features.Feature{
	{Name: SupportOIDC, Channel: features.FeatureChannelExperimental},
}
