package prerun

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"time"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	"golang.org/x/mod/semver"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	pkgversion "github.com/agentgateway/agentgateway/controller/pkg/version"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const controllerLabelSelector = "app.kubernetes.io/name=agentgateway"

// CheckVersionMismatch fetches the controller version via port-forward and emits a warning
// to stderr if the major or minor version differs from the client. Errors are non-fatal.
func CheckVersionMismatch(ctx context.Context, namespace string, stderr io.Writer) {
	serverVersion, err := fetchControllerVersion(ctx, namespace)
	if err != nil {
		fmt.Fprintf(stderr, "warning: could not check controller version: %v\n", err)
		return
	}

	clientVersion := pkgversion.Version
	if clientVersion == "" || clientVersion == pkgversion.UndefinedVersion {
		return
	}

	// Normalize to semver canonical form (add 'v' prefix if missing).
	sv := canonicalize(serverVersion)
	cv := canonicalize(clientVersion)

	if !semver.IsValid(sv) || !semver.IsValid(cv) {
		return
	}

	if semver.Major(sv) != semver.Major(cv) || semver.MajorMinor(sv) != semver.MajorMinor(cv) {
		fmt.Fprintf(stderr, "warning: agctl version (%s) does not match controller version (%s); some commands may not work as expected\n",
			clientVersion, serverVersion)
	}
}

func canonicalize(v string) string {
	if len(v) > 0 && v[0] != 'v' {
		return "v" + v
	}
	return v
}

func fetchControllerVersion(ctx context.Context, namespace string) (string, error) {
	if namespace == "" {
		var err error
		namespace, err = kubeutil.LoadNamespace("")
		if err != nil {
			return "", fmt.Errorf("resolve namespace: %w", err)
		}
	}

	kubeClient, err := kubeutil.NewCLIClient()
	if err != nil {
		return "", fmt.Errorf("build kube client: %w", err)
	}

	podName, podNamespace, err := findControllerPod(ctx, kubeClient.Kube(), namespace)
	if err != nil {
		return "", err
	}

	adminPort := int(wellknown.AdminPort)
	body, err := kubeClient.EnvoyDoWithPort(ctx, podName, podNamespace, "GET", "version", adminPort)
	if err != nil {
		return "", fmt.Errorf("GET /version on %s/%s: %w", podNamespace, podName, err)
	}

	return parseVersionField(body)
}

func findControllerPod(ctx context.Context, kube kubernetes.Interface, namespace string) (string, string, error) {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()

	// Search in the provided namespace first, then fall back to all namespaces.
	for _, ns := range []string{namespace, ""} {
		deployments, err := kube.AppsV1().Deployments(ns).List(ctx, metav1.ListOptions{
			LabelSelector: controllerLabelSelector,
		})
		if err != nil {
			continue
		}
		for _, d := range deployments.Items {
			pods, err := kube.CoreV1().Pods(d.Namespace).List(ctx, metav1.ListOptions{
				LabelSelector: controllerLabelSelector,
			})
			if err != nil || len(pods.Items) == 0 {
				continue
			}
			for _, p := range pods.Items {
				if p.Status.Phase == "Running" {
					return p.Name, p.Namespace, nil
				}
			}
		}
	}

	return "", "", fmt.Errorf("no running controller pod found with label %q", controllerLabelSelector)
}

// versionResponse matches the JSON returned by GET /version on the admin server.
type versionResponse struct {
	Version string `json:"version"`
}

func parseVersionField(body []byte) (string, error) {
	var resp versionResponse
	if err := json.Unmarshal(body, &resp); err != nil {
		return "", fmt.Errorf("parse version response: %w", err)
	}
	return resp.Version, nil
}
