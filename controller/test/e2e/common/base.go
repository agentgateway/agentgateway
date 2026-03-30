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
	gatewayForwards  = map[types.NamespacedName]*gatewayPortForwardState{}
	baseInstallation *e2e.TestInstallation
	baseContext      context.Context
)

type gatewayPortForwardState struct {
	address    string
	portMap    map[int]int
	forwarders []portforward.PortForwarder
}

// ResolveGatewayAddress returns a reachable gateway address for e2e traffic.
// If USE_PORTFORWARD is set, tests use a local port-forward; otherwise, they use the LoadBalancer address.
func ResolveGatewayAddress(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) string {
	if !shouldUsePortForward() {
		return installation.Assertions.EventuallyGatewayAddress(ctx, name.Name, name.Namespace)
	}

	gatewayAddressMu.Lock()
	defer gatewayAddressMu.Unlock()
	state, err := ensureGatewayPortForwardsLocked(ctx, installation, name)
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
	return state.address
}

// ResolveGatewayPort resolves the local forwarded port for a remote gateway service port.
// If USE_PORTFORWARD is not set, it returns remotePort unchanged.
func ResolveGatewayPort(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName, remotePort int) int {
	if !shouldUsePortForward() {
		return remotePort
	}

	gatewayAddressMu.Lock()
	defer gatewayAddressMu.Unlock()

	state, err := ensureGatewayPortForwardsLocked(ctx, installation, name)
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
	if localPort, ok := state.portMap[remotePort]; ok {
		return localPort
	}
	return remotePort
}

func shouldUsePortForward() bool {
	_, set := os.LookupEnv("USE_PORTFORWARD")
	return set
}

func ensureGatewayPortForwardsLocked(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) (*gatewayPortForwardState, error) {
	if state, ok := gatewayForwards[name]; ok {
		if state.isHealthy() {
			return state, nil
		}
		state.close()
		delete(gatewayForwards, name)
	}

	state, err := setupGatewayPortForwards(ctx, installation, name)
	if err != nil {
		return nil, err
	}
	gatewayForwards[name] = state
	return state, nil
}

func setupGatewayPortForwards(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) (*gatewayPortForwardState, error) {
	svc := &corev1.Service{}
	if err := installation.ClusterContext.Client.Get(ctx, name, svc); err != nil {
		return nil, fmt.Errorf("failed to get gateway service %s/%s: %w", name.Namespace, name.Name, err)
	}
	if len(svc.Spec.Ports) == 0 {
		return nil, fmt.Errorf("gateway service %s/%s has no ports", name.Namespace, name.Name)
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
			return nil, fmt.Errorf("failed to port-forward service %s/%s on port %d: %w", name.Namespace, name.Name, remotePort, err)
		}
		_, localPort, err := net.SplitHostPort(forwarder.Address())
		if err != nil {
			for _, started := range forwarders {
				started.Close()
			}
			return nil, fmt.Errorf("failed to parse local port-forward address %q for service %s/%s port %d: %w", forwarder.Address(), name.Namespace, name.Name, remotePort, err)
		}
		parsedLocalPort, err := strconv.Atoi(localPort)
		if err != nil {
			for _, started := range forwarders {
				started.Close()
			}
			return nil, fmt.Errorf("failed to parse local port-forward port %q for service %s/%s port %d: %w", localPort, name.Namespace, name.Name, remotePort, err)
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

	return &gatewayPortForwardState{
		address:    defaultAddress,
		portMap:    portMap,
		forwarders: forwarders,
	}, nil
}

func (s *gatewayPortForwardState) close() {
	for _, forwarder := range s.forwarders {
		forwarder.Close()
	}
}

func (s *gatewayPortForwardState) isHealthy() bool {
	if s == nil || s.address == "" {
		return false
	}
	if !addressReachable(s.address) {
		return false
	}
	for _, localPort := range s.portMap {
		if !addressReachable(net.JoinHostPort("127.0.0.1", strconv.Itoa(localPort))) {
			return false
		}
	}
	return true
}

func addressReachable(address string) bool {
	conn, err := net.DialTimeout("tcp", address, time.Second)
	if err != nil {
		return false
	}
	_ = conn.Close()
	return true
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
	if shouldUsePortForward() && g.NamespacedName.Name != "" && baseInstallation != nil {
		return ResolveGatewayAddress(resolveBaseGatewayContext(), baseInstallation, g.NamespacedName)
	}
	return g.Address
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
