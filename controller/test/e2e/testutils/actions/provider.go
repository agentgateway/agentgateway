//go:build e2e

package actions

import (
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/helmutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/cluster"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/install"
)

// Provider is the entity that creates actions.
// These actions are executed against a running installation of kgateway, within a Kubernetes Cluster.
// This provider is just a wrapper around sub-providers, so it exposes methods to access those providers
type Provider struct {
	kubeCli *kubectl.Cli
	helmCli *helmutils.Client

	installContext *install.Context
}

// NewActionsProvider returns an Provider
func NewActionsProvider() *Provider {
	return &Provider{
		kubeCli:        nil,
		helmCli:        helmutils.NewClient(),
		installContext: nil,
	}
}

// WithClusterContext sets the provider to point to the provided cluster
func (p *Provider) WithClusterContext(clusterContext *cluster.Context) *Provider {
	p.kubeCli = clusterContext.Cli
	return p
}

// WithInstallContext sets the provider to point to the provided kgateway installation
func (p *Provider) WithInstallContext(installContext *install.Context) *Provider {
	p.installContext = installContext
	return p
}

func (p *Provider) Kubectl() *kubectl.Cli {
	return p.kubeCli
}

func (p *Provider) Helm() *helmutils.Client {
	return p.helmCli
}
