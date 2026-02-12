package helpers

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/onsi/ginkgo/v2"
	"golang.org/x/sync/errgroup"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/kubectl"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/threadsafe"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

// StandardKgatewayDumpOnFail creates a dump of the kubernetes state and certain envoy data from
// the admin interface when a test fails.
// Look at `KubeDumpOnFail` && `EnvoyDumpOnFail` for more details
func StandardKgatewayDumpOnFail(outLog io.Writer, kubectlCli *kubectl.Cli, outDir string, namespaces []string) {
	if os.Getenv(testutils.SkipDump) == "true" {
		return
	}
	fmt.Printf("Test failed. Dumping state from %s...\n", strings.Join(namespaces, ", "))

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	// only wipe at the start of the dump
	wipeOutDir(outDir)

	KubeDumpOnFail(ctx, kubectlCli, outLog, outDir, namespaces)

	fmt.Printf("Test failed. Logs and cluster state are available in %s\n", outDir)
}

// KubeDumpOnFail creates a small dump of the kubernetes state when a test fails.
// This is useful for debugging test failures.
// The dump includes:
// - docker state
// - process state
// - kubernetes state
// - logs from all pods in the given namespaces
// - yaml representations of all kgateway CRs in the given namespaces
func KubeDumpOnFail(ctx context.Context, kubectlCli *kubectl.Cli, outLog io.Writer, outDir string,
	namespaces []string,
) {
	t0 := time.Now()
	setupOutDir(outDir)

	recordKubeState(ctx, kubectlCli, fileAtPath(filepath.Join(outDir, "kube-state.log")))

	recordKubeDump(outDir, namespaces...)

	fmt.Printf("Finished dumping kubernetes state (%v)\n", time.Since(t0))
}

func recordKubeState(ctx context.Context, kubectlCli *kubectl.Cli, f *os.File) {
	defer f.Close()

	resourcesToGet := []string{
		// Kubernetes resources
		"secrets",
		"services",
		"pods",
		"deployments",
		"configmaps",
		// Agentgateway
		"agentgatewaybackends.agentgateway.dev",
		"agentgatewayparameters.agentgateway.dev",
		"agentgatewaypolicies.agentgateway.dev",
		// Kube GW API resources
		"backendtlspolicies.gateway.networking.k8s.io",
		"gatewayclasses.gateway.networking.k8s.io",
		"gateways.gateway.networking.k8s.io",
		"grpcroutes.gateway.networking.k8s.io",
		"httproutes.gateway.networking.k8s.io",
		"referencegrants.gateway.networking.k8s.io",
		"tcproutes.gateway.networking.k8s.io",
		"tlsroutes.gateway.networking.k8s.io",
		"udproutes.gateway.networking.k8s.io",
		"xbackendtrafficpolicies.gateway.networking.x-k8s.io",
		"xlistenersets.gateway.networking.x-k8s.io",
		"xmeshes.gateway.networking.x-k8s.io",
		// GIE
		"inferencepools.inference.networking.k8s.io",
	}

	f.WriteString("*** Kube resources ***\n")
	err := kubectlCli.RunCommandToWriters(ctx, f, f, "get", strings.Join(resourcesToGet, ","), "-A", "-owide")
	if err != nil {
		f.WriteString("*** Unable to get kube resources ***. Reason: " + err.Error() + " \n")
	}
}

func recordKubeDump(outDir string, namespaces ...string) {
	g := errgroup.Group{}
	// for each namespace, create a namespace directory that contains...
	for _, ns := range namespaces {
		// ...a pod logs subdirectoy
		g.Go(func() error {
			return recordPods(filepath.Join(outDir, ns, "_pods"), ns)
		})
	}
	if err := g.Wait(); err != nil {
		fmt.Printf("error recording pod logs: %v, \n", err)
	}
}

// recordPods records logs from each pod to <output-dir>/$namespace/pods/$pod.log
func recordPods(podDir, namespace string) error {
	pods, err := kubeList(namespace, "pod")
	if err != nil {
		return err
	}

	var errs []error

	if err := os.MkdirAll(podDir, os.ModePerm); err != nil {
		return err
	}
	g := errgroup.Group{}
	for _, pod := range pods {
		g.Go(func() error {
			logs, errOutput, err := kubeLogs(namespace, pod)
			// store any error running the log command to return later
			// the error represents the cause of the failure, and should be bubbled up
			// we will still try to get logs for other pods even if this one returns an error
			if err != nil {
				errs = append(errs, err)
			}
			// write any log output to the standard file
			if logs != "" {
				f := fileAtPath(filepath.Join(podDir, pod+".log"))
				defer f.Close()
				f.WriteString(logs)
			}
			// write any error output to the error file
			// this will consist of the combined stdout and stderr of the command
			if errOutput != "" {
				f := fileAtPath(filepath.Join(podDir, pod+"-error.log"))
				defer f.Close()
				f.WriteString(errOutput)
			}

			return nil
		})
	}
	g.Wait()

	return errors.Join(errs...)
}

// kubeLogs runs $(kubectl -n $namespace logs $pod --all-containers) and returns the string result
func kubeLogs(namespace string, pod string) (string, string, error) {
	args := []string{"-n", namespace, "logs", pod, "--all-containers"}
	return kubeExecute(args)
}

func kubeExecute(args []string) (string, string, error) {
	cli := kubectl.NewCli().WithReceiver(ginkgo.GinkgoWriter)

	var outLocation threadsafe.Buffer
	runError := cli.Command(context.Background(), args...).WithStdout(&outLocation).Run()
	if runError != nil {
		return outLocation.String(), runError.OutputString(), runError.Cause()
	}

	return outLocation.String(), "", nil
}

// kubeList runs $(kubectl -n $namespace $target) and returns a slice of kubernetes object names
func kubeList(namespace string, target string) ([]string, error) {
	args := []string{"-n", namespace, "get", target}
	lines, _, err := kubeExecute(args)
	if err != nil {
		return nil, err
	}

	var toReturn []string
	for line := range strings.SplitSeq(strings.TrimSuffix(lines, "\n"), "\n") {
		if strings.HasPrefix(line, "NAME") || strings.HasPrefix(line, "No resources found") {
			continue // skip header line and cases where there are no resources
		}
		if split := strings.Split(line, " "); len(split) > 1 {
			toReturn = append(toReturn, split[0])
		}
	}
	return toReturn, nil
}

func wipeOutDir(outDir string) {
	err := os.RemoveAll(outDir)
	if err != nil {
		fmt.Printf("error wiping out directory: %f\n", err)
	}
}

// setupOutDir forcibly deletes/creates the output directory
func setupOutDir(outdir string) {
	err := os.MkdirAll(outdir, os.ModePerm)
	if err != nil {
		fmt.Printf("error creating log directory: %f\n", err)
	}
}

// fileAtPath creates a file at the given path, and returns the file object
func fileAtPath(path string) *os.File {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0600)
	if err != nil {
		fmt.Printf("unable to openfile: %f\n", err)
	}
	return f
}
