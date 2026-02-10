package deployer

import (
	"k8s.io/apimachinery/pkg/runtime"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/deployer"
)

func NewGatewayDeployer(agwControllerName, agwGatewayClassName string, scheme *runtime.Scheme, client apiclient.Client, gwParams *GatewayParameters, opts ...deployer.Option) (*deployer.Deployer, error) {
	agentgatewayChart, err := LoadAgentgatewayChart()
	if err != nil {
		return nil, err
	}
	return deployer.NewDeployerWithMultipleCharts(
		agwControllerName, agwGatewayClassName, scheme, client, agentgatewayChart, gwParams, GatewayReleaseNameAndNamespace, opts...), nil
}
