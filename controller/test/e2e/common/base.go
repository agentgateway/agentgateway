//go:build e2e

package common

import (
	"context"
	"fmt"
	"log"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/portforward"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func SetupBaseConfig(ctx context.Context, t *testing.T, installation *e2e.TestInstallation, manifests ...string) {
	err := installation.ClusterContext.IstioClient.ApplyYAMLFiles("", manifests...)
	assert.NoError(t, err)
}

func SetupBaseGateway(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) {
	baseInstallation = installation
	baseContext = ctx
	BaseGateway = Gateway{
		NamespacedName: name,
		Address:        ResolveGatewayAddress(ctx, installation, name),
	}
}

var (
	gatewayAddressMu sync.Mutex
	gatewayAddresses = map[types.NamespacedName]string{}
	gatewayPorts     = map[types.NamespacedName]map[int]int{}
	baseInstallation *e2e.TestInstallation
	baseContext      context.Context
)

// ResolveGatewayAddress returns a reachable gateway address for e2e traffic.
// If USE_PORTFORWARD is set, tests use a local port-forward; otherwise, they use the LoadBalancer address.
func ResolveGatewayAddress(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) string {
	if !shouldUsePortForward() {
		return installation.Assertions.EventuallyGatewayAddress(ctx, name.Name, name.Namespace)
	}

	gatewayAddressMu.Lock()
	defer gatewayAddressMu.Unlock()
	if addr, ok := gatewayAddresses[name]; ok {
		return addr
	}

	addr, portMap, err := setupGatewayPortForwards(ctx, installation, name)
	if err != nil {
		log.Printf(
			"WARN: USE_PORTFORWARD is set but port-forward setup failed for Gateway %s/%s: %v; falling back to LoadBalancer address",
			name.Namespace,
			name.Name,
			err,
		)
		// Do not cache the fallback LB address. Keep retrying port-forward resolution on subsequent calls.
		return installation.Assertions.EventuallyGatewayAddress(ctx, name.Name, name.Namespace)
	}
	gatewayAddresses[name] = addr
	gatewayPorts[name] = portMap
	return addr
}

// ResolveGatewayPort resolves the local forwarded port for a remote gateway service port.
// If USE_PORTFORWARD is not set, it returns remotePort unchanged.
func ResolveGatewayPort(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName, remotePort int) int {
	if !shouldUsePortForward() {
		return remotePort
	}

	gatewayAddressMu.Lock()
	defer gatewayAddressMu.Unlock()

	if ports, ok := gatewayPorts[name]; ok {
		if localPort, ok := ports[remotePort]; ok {
			return localPort
		}
	}
	// Ensure cached port-forwards are initialized for this gateway.
	if _, ok := gatewayAddresses[name]; !ok {
		addr, portMap, err := setupGatewayPortForwards(ctx, installation, name)
		if err != nil {
			log.Printf(
				"WARN: USE_PORTFORWARD is set but port-forward setup failed for Gateway %s/%s: %v; using remote port %d",
				name.Namespace,
				name.Name,
				err,
				remotePort,
			)
			return remotePort
		}
		gatewayAddresses[name] = addr
		gatewayPorts[name] = portMap
	}
	if ports, ok := gatewayPorts[name]; ok {
		if localPort, ok := ports[remotePort]; ok {
			return localPort
		}
	}
	return remotePort
}

func shouldUsePortForward() bool {
	_, set := os.LookupEnv("USE_PORTFORWARD")
	return set
}

func setupGatewayPortForwards(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) (string, map[int]int, error) {
	svc := &corev1.Service{}
	if err := installation.ClusterContext.Client.Get(ctx, name, svc); err != nil {
		return "", nil, fmt.Errorf("failed to get gateway service %s/%s: %w", name.Namespace, name.Name, err)
	}
	if len(svc.Spec.Ports) == 0 {
		return "", nil, fmt.Errorf("gateway service %s/%s has no ports", name.Namespace, name.Name)
	}

	forwarders := make([]portforward.PortForwarder, 0, len(svc.Spec.Ports))
	portMap := make(map[int]int, len(svc.Spec.Ports))
	defaultAddress := ""
	for _, port := range svc.Spec.Ports {
		remotePort := int(port.Port)
		options := []portforward.Option{
			portforward.WithService(name.Name, name.Namespace),
			portforward.WithRemotePort(remotePort),
		}

		forwarder, err := installation.Actions.Kubectl().StartPortForward(ctx, options...)
		if err != nil {
			for _, started := range forwarders {
				started.Close()
			}
			return "", nil, fmt.Errorf("failed to port-forward service %s/%s on port %d: %w", name.Namespace, name.Name, remotePort, err)
		}
		_, localPort, err := net.SplitHostPort(forwarder.Address())
		if err != nil {
			for _, started := range forwarders {
				started.Close()
			}
			return "", nil, fmt.Errorf("failed to parse local port-forward address %q for service %s/%s port %d: %w", forwarder.Address(), name.Namespace, name.Name, remotePort, err)
		}
		parsedLocalPort, err := strconv.Atoi(localPort)
		if err != nil {
			for _, started := range forwarders {
				started.Close()
			}
			return "", nil, fmt.Errorf("failed to parse local port-forward port %q for service %s/%s port %d: %w", localPort, name.Namespace, name.Name, remotePort, err)
		}
		forwarders = append(forwarders, forwarder)
		portMap[remotePort] = parsedLocalPort

		if defaultAddress == "" || port.Port == 80 || strings.EqualFold(port.Name, "http") {
			defaultAddress = forwarder.Address()
		}
	}

	go func() {
		<-ctx.Done()
		for _, forwarder := range forwarders {
			forwarder.Close()
		}
	}()

	return defaultAddress, portMap, nil
}

type Gateway struct {
	types.NamespacedName
	Address string
}

var BaseGateway Gateway

func (g *Gateway) Send(t *testing.T, match *matchers.HttpResponse, opts ...curl.Option) {
	resp := g.SendWithResponse(t, match, opts...)
	_ = resp.Body.Close()
}

func (g *Gateway) SendWithResponse(t *testing.T, match *matchers.HttpResponse, opts ...curl.Option) http.Response {
	address := g.ResolvedAddress()
	fullOpts := append(GatewayAddressOptions(address), opts...)
	var passedRes http.Response
	retry.UntilSuccessOrFail(t, func() error {
		r, err := curl.ExecuteRequest(fullOpts...)
		if err != nil {
			return err
		}
		mm := matchers.HaveHttpResponse(match)
		success, err := mm.Match(r)
		if err != nil {
			r.Body.Close()
			return err
		}
		if !success {
			r.Body.Close()
			return fmt.Errorf("match failed: %v", mm.FailureMessage(r))
		}
		passedRes = *r
		return nil
	}, retry.Timeout(time.Second*30))
	return passedRes
}

func (g *Gateway) ResolvedAddress() string {
	address := g.Address
	if shouldUsePortForward() && g.NamespacedName.Name != "" && !addressHasPort(address) && baseInstallation != nil {
		return ResolveGatewayAddress(resolveBaseGatewayContext(), baseInstallation, g.NamespacedName)
	}
	return address
}

func (g *Gateway) PortForRemote(remotePort int) int {
	if shouldUsePortForward() && g.NamespacedName.Name != "" && baseInstallation != nil {
		return ResolveGatewayPort(resolveBaseGatewayContext(), baseInstallation, g.NamespacedName, remotePort)
	}
	return remotePort
}

func resolveBaseGatewayContext() context.Context {
	if baseContext != nil {
		return baseContext
	}
	return context.Background()
}

func GatewayAddressOptions(address string) []curl.Option {
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return []curl.Option{curl.WithHost(address)}
	}
	if strings.EqualFold(host, "localhost") {
		host = "127.0.0.1"
	}
	parsedPort, err := strconv.Atoi(port)
	if err != nil {
		return []curl.Option{curl.WithHost(address)}
	}
	return []curl.Option{curl.WithHost(host), curl.WithPort(parsedPort)}
}

func addressHasPort(address string) bool {
	_, _, err := net.SplitHostPort(address)
	return err == nil
}
