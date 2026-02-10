//go:build e2e

package assertions

import (
	"context"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

const (
	// KgatewayLabelSelector is the label selector for kgateway pods
	KgatewayLabelSelector = "app.kubernetes.io/name=kgateway"
	// AgentgatewayLabelSelector is the label selector for agentgateway pods
	AgentgatewayLabelSelector = "app.kubernetes.io/name=agentgateway"
)

// getChartLabelSelector returns the appropriate label selector based on the chart type
func (p *Provider) getChartLabelSelector() string {
	return AgentgatewayLabelSelector
}

func (p *Provider) EventuallyGatewayInstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsRunning(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: p.getChartLabelSelector(),
		})
}

func (p *Provider) EventuallyGatewayUninstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsNotExist(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: p.getChartLabelSelector(),
		})
}

func (p *Provider) EventuallyGatewayUpgradeSucceeded(ctx context.Context, version string) {
	p.expectInstallContextDefined()

	p.EventuallyPodsRunning(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: p.getChartLabelSelector(),
		})
}

// EventuallyKgatewayInstallSucceeded verifies that the kgateway chart installation has succeeded.
// This is useful when testing with both kgateway and agentgateway charts installed.
func (p *Provider) EventuallyKgatewayInstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsRunning(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: KgatewayLabelSelector,
		})
}

// EventuallyKgatewayUninstallSucceeded verifies that the kgateway chart has been uninstalled.
// This is useful when testing with both kgateway and agentgateway charts.
func (p *Provider) EventuallyKgatewayUninstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsNotExist(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: KgatewayLabelSelector,
		})
}

// EventuallyKgatewayUpgradeSucceeded verifies that the kgateway chart upgrade has succeeded.
// This is useful when testing with both kgateway and agentgateway charts installed.
func (p *Provider) EventuallyKgatewayUpgradeSucceeded(ctx context.Context, version string) {
	p.expectInstallContextDefined()

	p.EventuallyPodsRunning(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: KgatewayLabelSelector,
		})
}

// EventuallyAgentgatewayInstallSucceeded verifies that the agentgateway chart installation has succeeded.
// This is useful when testing with both kgateway and agentgateway charts installed.
func (p *Provider) EventuallyAgentgatewayInstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsRunning(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: AgentgatewayLabelSelector,
		})
}

// EventuallyAgentgatewayUninstallSucceeded verifies that the agentgateway chart has been uninstalled.
// This is useful when testing with both kgateway and agentgateway charts.
func (p *Provider) EventuallyAgentgatewayUninstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsNotExist(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: AgentgatewayLabelSelector,
		})
}

// EventuallyAgentgatewayUpgradeSucceeded verifies that the agentgateway chart upgrade has succeeded.
// This is useful when testing with both kgateway and agentgateway charts installed.
func (p *Provider) EventuallyAgentgatewayUpgradeSucceeded(ctx context.Context, version string) {
	p.expectInstallContextDefined()

	p.EventuallyPodsRunning(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: AgentgatewayLabelSelector,
		})
}
