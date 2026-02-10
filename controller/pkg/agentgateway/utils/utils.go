package utils

import (
	"fmt"
	"strconv"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pkg/ptr"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

// SingularLLMProviderSubBackendName is the name of the sub-backend for singular LLM providers.
// If the Backend is ns/foo, the sub-backend will be ns/foo/backend
const SingularLLMProviderSubBackendName = "backend"

// InternalGatewayName returns the name of the internal Gateway corresponding to the
// specified gwv1-api gwv1 and listener. If the listener is not specified, returns internal name without listener.
// Format: gwNs/gwName.listener
func InternalGatewayName(gwNamespace, gwName, lName string) string {
	if lName == "" {
		return fmt.Sprintf("%s/%s", gwNamespace, gwName)
	}
	return fmt.Sprintf("%s/%s.%s", gwNamespace, gwName, lName)
}

// InternalRouteRuleKey returns the name of the internal Route Rule corresponding to the
// specified route. If ruleName is not specified, returns the internal name without the route rule.
// Format: routeNs/routeName.ruleName
func InternalRouteRuleKey(routeNamespace, routeName, ruleName string) string {
	if ruleName == "" {
		return fmt.Sprintf("%s/%s", routeNamespace, routeName)
	}
	return fmt.Sprintf("%s/%s.%s", routeNamespace, routeName, ruleName)
}

// InternalMCPStaticBackendName returns the name of the internal MCP Static Backend corresponding to the
// specified backend and target.
// Format: backendNamespace/backendName/targetName
func InternalMCPStaticBackendName(backendNamespace, backendName, targetName string) string {
	return backendNamespace + "/" + backendName + "/" + targetName
}

// InternalBackendKey returns the name of the internal Backend corresponding to the
// specified backend and target.
// Format: backendNamespace/backendName when targetName is empty, otherwise backendNamespace/backendName/targetName
func InternalBackendKey(backendNamespace, backendName, targetName string) string {
	name := backendNamespace + "/" + backendName
	if targetName != "" {
		name += "/" + targetName
	}
	return name
}

func ListenerName(namespace, name string, listener string) *api.ListenerName {
	return &api.ListenerName{
		GatewayName:      name,
		GatewayNamespace: namespace,
		ListenerName:     listener,
		ListenerSet:      nil,
	}
}

func RouteName[T ~string](kind string, namespace, name string, routeRule *T) *api.RouteName {
	var ls *string
	if routeRule != nil {
		ls = ptr.Of((string)(*routeRule))
	}
	return &api.RouteName{
		Name:      name,
		Namespace: namespace,
		RuleName:  ls,
		Kind:      kind,
	}
}
func ServiceTarget[T ~string](namespace, name string, port *T) *api.PolicyTarget_Service {
	hostname := fmt.Sprintf("%s.%s.svc.%s", name, namespace, kubeutils.GetClusterDomainName())
	var ls *string
	if port != nil {
		ls = ptr.Of((string)(*port))
	}
	return ServiceTargetWithHostname(namespace, hostname, ls)
}

func InferencePoolTarget[T ~string](namespace, name string, port *T) *api.PolicyTarget_Service {
	hostname := fmt.Sprintf("%s.%s.inference.%s", name, namespace, kubeutils.GetClusterDomainName())
	var ls *string
	if port != nil {
		ls = ptr.Of((string)(*port))
	}
	return ServiceTargetWithHostname(namespace, hostname, ls)
}

func ServiceTargetWithHostname(namespace, hostname string, port *string) *api.PolicyTarget_Service {
	var portNum *uint32
	if port != nil {
		parsed, _ := strconv.Atoi(*port)
		portNum = ptr.Of(uint32(parsed)) // nolint:gosec // G115: kubebuilder validation ensures safe for uint32
	}
	return &api.PolicyTarget_Service{
		Service: &api.PolicyTarget_ServiceTarget{
			Hostname:  hostname,
			Namespace: namespace,
			Port:      portNum,
		},
	}
}

func GatewayTarget[T ~string](namespace, name string, listener *T) *api.PolicyTarget_Gateway {
	var ls *string
	if listener != nil {
		ls = ptr.Of((string)(*listener))
	}
	return &api.PolicyTarget_Gateway{
		Gateway: &api.PolicyTarget_GatewayTarget{
			Name:      name,
			Namespace: namespace,
			Listener:  ls,
		},
	}
}

func RouteTarget[T ~string](namespace, name, kind string, ruleName *T) *api.PolicyTarget_Route {
	var ls *string
	if ruleName != nil {
		ls = ptr.Of((string)(*ruleName))
	}
	return &api.PolicyTarget_Route{
		Route: &api.PolicyTarget_RouteTarget{
			Name:      name,
			Namespace: namespace,
			RouteRule: ls,
			Kind:      kind,
		},
	}
}

func BackendTarget[T ~string](backendNamespace, backendName string, section *T) *api.PolicyTarget_Backend {
	var ls *string
	if section != nil {
		ls = ptr.Of((string)(*section))
	}
	return &api.PolicyTarget_Backend{
		Backend: &api.PolicyTarget_BackendTarget{
			Name:      backendName,
			Namespace: backendNamespace,
			Section:   ls,
		},
	}
}
