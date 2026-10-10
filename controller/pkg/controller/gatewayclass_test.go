package controller

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"k8s.io/apimachinery/pkg/util/sets"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
)

func TestGatewayClassReconcilerUsesConfiguredControllerName(t *testing.T) {
	const (
		gatewayClassName = "agentgateway-secondary"
		controllerName   = "test.agentgateway.dev/secondary"
	)

	reconciler := &gatewayClassReconciler{
		defaultControllerName: controllerName,
		classInfo: map[string]*deployer.GatewayClassInfo{
			gatewayClassName: {},
		},
	}

	gatewayClass := reconciler.buildDesiredGatewayClass(gatewayClassName, reconciler.classInfo[gatewayClassName])

	assert.Equal(t, gatewayClassName, gatewayClass.Name)
	assert.Equal(t, gwv1.GatewayController(controllerName), gatewayClass.Spec.ControllerName)
	assert.True(t, isOurGatewayClass(gatewayClass, sets.New(controllerName)))
	assert.False(t, isOurGatewayClass(gatewayClass, sets.New("test.agentgateway.dev/primary")))
}
