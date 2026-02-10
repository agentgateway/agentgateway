package deployer

import (
	"errors"
	"fmt"
)

var (
	GatewayParametersError              = errors.New("could not retrieve GatewayParameters")
	GetGatewayParametersForGatewayError = func(err error, gwpNamespace, gwpName, gwNamespace, gwName, resourceType string) error {
		return fmt.Errorf("(%s.%s) for %s (%s.%s): %w",
			gwpNamespace, gwpName, resourceType, gwNamespace, gwName, fmt.Errorf("%s: %w", GatewayParametersError.Error(), err))
	}
	GetGatewayParametersForGatewayClassError = func(err error, gwpNamespace, gwpName, gwcName, resourceType string) error {
		return fmt.Errorf("(%s.%s) for %s (%s): %w",
			gwpNamespace, gwpName, resourceType, gwcName, fmt.Errorf("%s: %w", GatewayParametersError.Error(), err))
	}
	NilDeployerInputsErr = errors.New("nil inputs to NewDeployer")
)
