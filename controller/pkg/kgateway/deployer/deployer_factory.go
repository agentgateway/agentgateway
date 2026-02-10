package deployer

import (
	"k8s.io/apimachinery/pkg/runtime"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
)

func NewGatewayDeployer(agwControllerName, agwGatewayClassName string, scheme *runtime.Scheme, client apiclient.Client, gwParams *GatewayParameters, opts ...deployer.Option) (*deployer.Deployer, error) {
	agentgatewayChart, err := LoadAgentgatewayChart()
	if err != nil {
		return nil, err
	}
	return deployer.NewDeployerWithMultipleCharts(
		agwControllerName, agwGatewayClassName, scheme, client, agentgatewayChart, gwParams, GatewayReleaseNameAndNamespace, opts...), nil
}
