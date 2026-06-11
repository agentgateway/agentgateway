package profile

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"time"

	"github.com/spf13/cobra"
	"istio.io/istio/pkg/kube"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
)

const (
	localForwardAddress = "127.0.0.1"
	localRuntimeAddress = "127.0.0.1"
	heapProfileTimeout  = 30 * time.Second
	profileTimeoutGrace = 10 * time.Second
)

var profileHTTPClient = &http.Client{}

type profileKind string

const (
	profileKindCPU  profileKind = "cpu"
	profileKindHeap profileKind = "heap"
)

type profileTarget struct {
	KubeClient   kube.CLIClient
	ResourceName string
	PodName      string
	PodNamespace string
	Local        bool
}

func run(cmd *cobra.Command, flags *profileFlags, args []string, kind profileKind) error {
	if err := flags.validate(kind, args); err != nil {
		return err
	}

	target, err := resolveProfileTarget(cmd.Context(), flags.namespace, flags.local, args)
	if err != nil {
		return err
	}

	adminAddress, closeAdmin, err := profileAdminAddress(target, flags.proxyAdminPort)
	if err != nil {
		return err
	}
	defer closeAdmin()

	outputFile := flags.outputFile
	if outputFile == "" {
		outputFile = defaultOutputFile(kind, flags.now())
	}

	if err := downloadProfile(cmd.Context(), adminAddress, kind, flags.seconds, outputFile); err != nil {
		return err
	}

	fmt.Fprintf(cmd.OutOrStdout(), "Wrote %s profile to %s\n", kind, outputFile)
	return nil
}

func resolveProfileTarget(ctx context.Context, namespaceOverride string, local bool, args []string) (*profileTarget, error) {
	if local {
		return &profileTarget{
			ResourceName: "localhost",
			Local:        true,
		}, nil
	}

	namespace, err := kubeutil.LoadNamespace(namespaceOverride)
	if err != nil {
		return nil, err
	}

	kubeClient, err := kubeutil.NewCLIClient()
	if err != nil {
		return nil, err
	}

	resourceName, err := kubeutil.ResolveResourceName(ctx, kubeClient, namespace, args)
	if err != nil {
		return nil, err
	}

	podName, podNamespace, err := kubeutil.ResolvePodForResource(kubeClient, resourceName, namespace)
	if err != nil {
		return nil, err
	}

	return &profileTarget{
		KubeClient:   kubeClient,
		ResourceName: resourceName,
		PodName:      podName,
		PodNamespace: podNamespace,
	}, nil
}

func profileAdminAddress(target *profileTarget, adminPort int) (string, func(), error) {
	if target.Local {
		return fmt.Sprintf("%s:%d", localRuntimeAddress, adminPort), func() {}, nil
	}

	adminForwarder, err := target.KubeClient.NewPortForwarder(target.PodName, target.PodNamespace, localForwardAddress, 0, adminPort)
	if err != nil {
		return "", nil, fmt.Errorf("failed to create admin port-forward for %s/%s: %w", target.PodNamespace, target.PodName, err)
	}
	if err := adminForwarder.Start(); err != nil {
		adminForwarder.Close()
		return "", nil, fmt.Errorf("failed to start admin port-forward for %s/%s: %w", target.PodNamespace, target.PodName, err)
	}

	return adminForwarder.Address(), adminForwarder.Close, nil
}

func profileURL(adminAddress string, kind profileKind, seconds int) string {
	u := url.URL{
		Scheme: "http",
		Host:   adminAddress,
	}
	switch kind {
	case profileKindCPU:
		u.Path = "/debug/pprof/profile"
		q := u.Query()
		q.Set("seconds", fmt.Sprintf("%d", seconds))
		u.RawQuery = q.Encode()
	case profileKindHeap:
		u.Path = "/debug/pprof/heap"
	}
	return u.String()
}

func downloadProfile(ctx context.Context, adminAddress string, kind profileKind, seconds int, outputFile string) error {
	client := *profileHTTPClient
	client.Timeout = profileTimeout(kind, seconds)

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, profileURL(adminAddress, kind, seconds), nil)
	if err != nil {
		return fmt.Errorf("failed to construct profile request: %w", err)
	}

	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("failed to fetch %s profile: %w", kind, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("%s profile returned %s: %s", kind, resp.Status, string(body))
	}

	file, err := os.Create(outputFile)
	if err != nil {
		return fmt.Errorf("failed to create output file %s: %w", outputFile, err)
	}

	if _, err := io.Copy(file, resp.Body); err != nil {
		closeErr := file.Close()
		_ = os.Remove(outputFile)
		if closeErr != nil {
			return fmt.Errorf("failed to write output file %s: %w", outputFile, closeErr)
		}
		return fmt.Errorf("failed to write output file %s: %w", outputFile, err)
	}
	if err := file.Close(); err != nil {
		_ = os.Remove(outputFile)
		return fmt.Errorf("failed to close output file %s: %w", outputFile, err)
	}
	return nil
}

func defaultOutputFile(kind profileKind, t time.Time) string {
	return fmt.Sprintf("agentgateway-%s-%s.pb.gz", kind, t.Format("20060102-150405"))
}

func profileTimeout(kind profileKind, seconds int) time.Duration {
	if kind == profileKindCPU {
		return time.Duration(seconds)*time.Second + profileTimeoutGrace
	}
	return heapProfileTimeout
}
