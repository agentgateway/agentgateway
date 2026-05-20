//go:build e2e

package actions

import (
	"context"
	"io"
	"time"

	"github.com/avast/retry-go/v4"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/helmutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/portforward"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/install"
)

// Provider is the entity that creates actions.
// These actions are executed against a running installation of agentgateway, within a Kubernetes Cluster.
// This provider is just a wrapper around sub-providers, so it exposes methods to access those providers
type Provider struct {
	helmCli     *helmutils.Client
	portForward *PortForwardProvider

	installContext *install.Context
}

type PortForwardProvider struct {
	kubeContext string
}

// NewActionsProvider returns an Provider
func NewActionsProvider() *Provider {
	return &Provider{
		helmCli:        helmutils.NewClient(),
		portForward:    &PortForwardProvider{},
		installContext: nil,
	}
}

// WithClusterContext sets the provider to point to the provided cluster
func (p *Provider) WithClusterContext(clusterContext *cluster.Context) *Provider {
	p.portForward.kubeContext = clusterContext.KubeContext
	return p
}

// WithInstallContext sets the provider to point to the provided agentgateway installation
func (p *Provider) WithInstallContext(installContext *install.Context) *Provider {
	p.installContext = installContext
	return p
}

func (p *Provider) Helm() *helmutils.Client {
	return p.helmCli
}

func (p *Provider) PortForward() *PortForwardProvider {
	return p.portForward
}

// Start creates and starts a port-forward. Callers are responsible for closing
// the returned PortForwarder.
func (p *PortForwardProvider) Start(ctx context.Context, options ...portforward.Option) (portforward.PortForwarder, error) {
	options = append([]portforward.Option{
		portforward.WithWriters(io.Discard, io.Discard),
		portforward.WithKubeContext(p.kubeContext),
	}, options...)

	forwarder := portforward.NewCliPortForwarder(options...)
	err := forwarder.Start(
		ctx,
		retry.LastErrorOnly(true),
		retry.Delay(250*time.Millisecond),
		retry.DelayType(retry.FixedDelay),
		retry.Attempts(60),
	)
	return forwarder, err
}
