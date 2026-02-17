//go:build e2e

package defaults

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/kubectl"
)

var (
	CurlPodExecOpt = kubectl.PodExecOptions{
		Name:      "curl",
		Namespace: "curl",
		Container: "curl",
	}

	WellKnownAppLabel = "app.kubernetes.io/name"

	AIGuardrailsWebhookManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "ai_guardrails_webhook.yaml")
)
