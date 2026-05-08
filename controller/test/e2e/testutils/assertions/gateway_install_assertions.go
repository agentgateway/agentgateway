//go:build e2e

package assertions

import (
	"context"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

const agentgatewayLabelSelector = "app.kubernetes.io/name=agentgateway"

func (p *Provider) EventuallyGatewayInstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsRunning(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: agentgatewayLabelSelector,
		})
}

func (p *Provider) EventuallyGatewayUninstallSucceeded(ctx context.Context) {
	p.expectInstallContextDefined()

	p.EventuallyPodsNotExist(ctx, p.installContext.InstallNamespace,
		metav1.ListOptions{
			LabelSelector: agentgatewayLabelSelector,
		})
}
