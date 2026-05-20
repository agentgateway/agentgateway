//go:build e2e

package defaults

import "github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/kubectl"

var (
	CurlPodExecOpt = kubectl.PodExecOptions{
		Name:      "curl",
		Namespace: "curl",
		Container: "curl",
	}

	WellKnownAppLabel = "app.kubernetes.io/name"
)
