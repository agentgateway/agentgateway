package deployer

import (
	"errors"
	"fmt"
	"net/netip"
	"regexp"

	"istio.io/istio/pkg/slices"
	"k8s.io/apimachinery/pkg/util/sets"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/collections"
)

var (
	// ErrMultipleAddresses is returned when multiple addresses are specified in Gateway.spec.addresses
	ErrMultipleAddresses = errors.New("multiple addresses given, only one address is supported")

	// ErrNoValidIPAddress is returned when no valid IP address is found in Gateway.spec.addresses
	ErrNoValidIPAddress = errors.New("IP address in Gateway.spec.addresses not valid")
)

// This file contains helper functions that generate helm values in the format needed
// by the deployer.

var ComponentLogLevelEmptyError = func(key string, value string) error {
	return fmt.Errorf("an empty key or value was provided in componentLogLevels: key=%s, value=%s", key, value)
}

// Extract the listener ports from a Gateway and corresponding listener sets. These will be used to populate:
// 1. the ports exposed on the envoy container
// 2. the ports exposed on the proxy service
func GetPortsValues(gw *collections.GatewayForDeployer) []HelmPort {
	gwPorts := []HelmPort{}

	// Add ports from Gateway listeners
	for _, port := range gw.Ports.List() {
		portName := GenerateListenerNameFromPort(port)
		if err := validateListenerPortForParent(port); err != nil {
			// skip invalid ports; statuses are handled in the translator
			logger.Error("skipping port", "gateway", gw.ResourceName(), "error", err)
			continue
		}
		gwPorts = AppendPortValue(gwPorts, port, portName)
	}

	return gwPorts
}

var agentGatewayReservedPorts = sets.New[int32](
	15020, // Metrics port
	15021, // Readiness port
	15000, // Envoy admin port
)

var ErrListenerPortReserved = fmt.Errorf("port is reserved")

func validateListenerPortForParent(port int32) error {
	if agentGatewayReservedPorts.Has(port) {
		return fmt.Errorf("invalid port %d in listener: %w",
			port, ErrListenerPortReserved)
	}
	return nil
}

func SanitizePortName(name string) string {
	nonAlphanumericRegex := regexp.MustCompile(`[^a-zA-Z0-9-]+`)
	str := nonAlphanumericRegex.ReplaceAllString(name, "-")
	doubleHyphen := regexp.MustCompile(`-{2,}`)
	str = doubleHyphen.ReplaceAllString(str, "-")

	// This is a kubernetes spec requirement.
	maxPortNameLength := 15
	if len(str) > maxPortNameLength {
		str = str[:maxPortNameLength]
	}
	return str
}

func AppendPortValue(gwPorts []HelmPort, port int32, name string) []HelmPort {
	if slices.IndexFunc(gwPorts, func(p HelmPort) bool { return *p.Port == port }) != -1 {
		return gwPorts
	}

	portName := SanitizePortName(name)
	protocol := "TCP"

	return append(gwPorts, HelmPort{
		Port:       &port,
		TargetPort: &port,
		Name:       &portName,
		Protocol:   &protocol,
	})
}

// GetLoadBalancerIPFromGatewayAddresses extracts the IP address from Gateway.spec.addresses.
// Returns the IP address if exactly one valid IP address is found, nil if no addresses are specified,
// or an error if more than one address is specified or no valid IP address is found.
func GetLoadBalancerIPFromGatewayAddresses(gw *gwv1.Gateway) (*string, error) {
	ipAddresses := slices.MapFilter(gw.Spec.Addresses, func(addr gwv1.GatewaySpecAddress) *string {
		if addr.Type == nil || *addr.Type == gwv1.IPAddressType {
			return &addr.Value
		}
		return nil
	})

	if len(ipAddresses) == 0 && len(gw.Spec.Addresses) != 0 {
		return nil, ErrNoValidIPAddress
	}

	if len(ipAddresses) == 0 {
		return nil, nil
	}
	if len(ipAddresses) > 1 {
		return nil, fmt.Errorf("%w: gateway %s/%s has %d addresses", ErrMultipleAddresses, gw.Namespace, gw.Name, len(gw.Spec.Addresses))
	}

	addr := ipAddresses[0]

	// Validate IP format
	parsedIP, err := netip.ParseAddr(addr)
	if err == nil && parsedIP.IsValid() {
		return &addr, nil
	}
	return nil, ErrNoValidIPAddress
}

// SetLoadBalancerIPFromGatewayForAgentgateway extracts the IP address from Gateway.spec.addresses
// and sets it on the AgentgatewayHelmService.
// Only sets the IP if exactly one valid IP address is found in Gateway.spec.addresses.
// Returns an error if more than one address is specified or no valid IP address is found.
// Note: Agentgateway services are always LoadBalancer type, so no service type check is needed.
func SetLoadBalancerIPFromGatewayForAgentgateway(gw *gwv1.Gateway, svc *AgentgatewayHelmService) error {
	ip, err := GetLoadBalancerIPFromGatewayAddresses(gw)
	if err != nil {
		return err
	}
	if ip != nil {
		svc.LoadBalancerIP = ip
	}
	return nil
}

func GenerateListenerNameFromPort(port gwv1.PortNumber) string {
	// Add a ~ to make sure the name won't collide with user provided names in other listeners
	return fmt.Sprintf("listener~%d", port)
}
