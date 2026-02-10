//go:build e2e

package defaults

import (
	"fmt"
	"path/filepath"

	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
)

var (
	ControllerLabelSelector = fmt.Sprintf("%s=%s", WellKnownAppLabel, "kgateway")

	CurlPodExecOpt = kubectl.PodExecOptions{
		Name:      "curl",
		Namespace: "curl",
		Container: "curl",
	}

	CurlPod = &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "curl",
			Namespace: "curl",
		},
	}

	CurlPodManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "curl_pod.yaml")

	CurlPodLabelSelector = fmt.Sprintf("%s=%s", WellKnownAppLabel, "curl")

	HttpEchoPod = &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "http-echo",
			Namespace: "http-echo",
		},
	}

	HttpEchoPodManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "http_echo.yaml")

	HttpbinManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "httpbin.yaml")

	HttpbinLabelSelector = fmt.Sprintf("%s=%s", WellKnownAppLabel, "httpbin")

	HttpbinDeployment = &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "httpbin",
			Namespace: "default",
		},
	}

	HttpbinService = &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "httpbin",
			Namespace: "default",
		},
	}

	TcpEchoPod = &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "tcp-echo",
			Namespace: "tcp-echo",
		},
	}

	TcpEchoPodManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "tcp_echo.yaml")

	NginxPod = &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "nginx",
			Namespace: "nginx",
		},
	}

	NginxSvc = &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "nginx",
			Namespace: "nginx",
		},
	}

	NginxPodManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "nginx_pod.yaml")

	NginxResponse = `<!DOCTYPE html>
<html>
<head>
<title>Welcome to nginx!</title>
<style>
html { color-scheme: light dark; }
body { width: 35em; margin: 0 auto;
font-family: Tahoma, Verdana, Arial, sans-serif; }
</style>
</head>
<body>
<h1>Welcome to nginx!</h1>
<p>If you see this page, the nginx web server is successfully installed and
working. Further configuration is required.</p>

<p>For online documentation and support please refer to
<a href="http://nginx.org/">nginx.org</a>.<br/>
Commercial support is available at
<a href="http://nginx.com/">nginx.com</a>.</p>

<p><em>Thank you for using nginx.</em></p>
</body>
</html>`

	WellKnownAppLabel = "app.kubernetes.io/name"

	AIGuardrailsWebhookManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "ai_guardrails_webhook.yaml")
)
