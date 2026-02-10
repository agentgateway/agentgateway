package helpers

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/onsi/ginkgo/v2"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/threadsafe"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
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
	setupOutDir(outDir)

	recordDockerState(fileAtPath(filepath.Join(outDir, "docker-state.log")))
	recordProcessState(fileAtPath(filepath.Join(outDir, "process-state.log")))
	recordKubeState(ctx, kubectlCli, fileAtPath(filepath.Join(outDir, "kube-state.log")))

	recordKubeDump(outDir, namespaces...)

	fmt.Printf("Finished dumping kubernetes state\n")
}

func recordDockerState(f *os.File) {
	defer f.Close()

	dockerCmd := exec.Command("docker", "ps")

	dockerState := &bytes.Buffer{}

	dockerCmd.Stdout = dockerState
	dockerCmd.Stderr = dockerState
	err := dockerCmd.Run()
	if err != nil {
		f.WriteString("*** Unable to get docker state ***. Reason: " + err.Error() + " \n")
		return
	}
	f.WriteString("*** Docker state ***\n")
	f.WriteString(dockerState.String() + "\n")
	f.WriteString("*** End Docker state ***\n")
}

func recordProcessState(f *os.File) {
	defer f.Close()

	psCmd := exec.Command("ps", "-auxf")

	psState := &bytes.Buffer{}

	psCmd.Stdout = psState
	psCmd.Stderr = psState
	err := psCmd.Run()
	if err != nil {
		f.WriteString("unable to get process state. Reason: " + err.Error() + " \n")
		return
	}
	f.WriteString("*** Process state ***\n")
	f.WriteString(psState.String() + "\n")
	f.WriteString("*** End Process state ***\n")
}

func recordKubeState(ctx context.Context, kubectlCli *kubectl.Cli, f *os.File) {
	defer f.Close()
	err := kubectlCli.RunCommandToWriters(ctx, f, f, "get", "all", "-A", "-o", "wide")
	if err != nil {
		f.WriteString(fmt.Sprintf("*** Unable to get kube state ***\nReason: %v", err))
	}

	resourcesToGet := []string{
		// Kubernetes resources
		"secrets",
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
		// kgateway resources
		"backends.gateway.kgateway.dev",
		"backendconfigpolicies.gateway.kgateway.dev",
		"directresponses.gateway.kgateway.dev",
		"gatewayextensions.gateway.kgateway.dev",
		"gatewayparameters.gateway.kgateway.dev",
		"httplistenerpolicies.gateway.kgateway.dev",
		"trafficpolicies.gateway.kgateway.dev",
	}

	f.WriteString("*** Kube resources ***\n")
	err = kubectlCli.RunCommandToWriters(ctx, f, f, "get", strings.Join(resourcesToGet, ","), "-A", "-owide")
	if err != nil {
		f.WriteString("*** Unable to get kube resources ***. Reason: " + err.Error() + " \n")
	}

	// Describe everything to identify the reason for issues such as Pods, LoadBalancers stuck in pending state
	// (insufficient resources, unable to acquire an IP), etc.
	// Ie: More context around the output of the previous command `kubectl get all -A`
	f.WriteString("*** Kube describe ***\n")
	err = kubectlCli.RunCommandToWriters(ctx, f, f, "describe", "all", "-A")
	if err != nil {
		f.WriteString("*** Unable to get kube describe ***. Reason: " + err.Error() + " \n")
	}

	f.WriteString("*** Kube endpoints ***\n")
	err = kubectlCli.RunCommandToWriters(ctx, f, f, "get", "endpoints", "-A")
	if err != nil {
		f.WriteString("*** Unable to get endpoint state ***. Reason: " + err.Error() + " \n")
	}
}

func recordKubeDump(outDir string, namespaces ...string) {
	// for each namespace, create a namespace directory that contains...
	for _, ns := range namespaces {
		// ...a pod logs subdirectoy
		if err := recordPods(filepath.Join(outDir, ns, "_pods"), ns); err != nil {
			fmt.Printf("error recording pod logs: %f, \n", err)
		}

		// ...and a subdirectory for each kgateway CRD with non-zero resources
		if err := recordCRs(filepath.Join(outDir, ns), ns); err != nil {
			fmt.Printf("error recording pod logs: %f, \n", err)
		}
	}
}

// recordPods records logs from each pod to <output-dir>/$namespace/pods/$pod.log
func recordPods(podDir, namespace string) error {
	pods, err := kubeList(namespace, "pod")
	if err != nil {
		return err
	}

	var errs []error

	for _, pod := range pods {
		if err := os.MkdirAll(podDir, os.ModePerm); err != nil {
			return err
		}

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
			f.WriteString(logs)
			f.Close()
		}
		// write any error output to the error file
		// this will consist of the combined stdout and stderr of the command
		if errOutput != "" {
			f := fileAtPath(filepath.Join(podDir, pod+"-error.log"))
			f.WriteString(errOutput)
			f.Close()
		}
	}

	return errors.Join(errs...)
}

// recordCRs records all unique CRs floating about to <output-dir>/$namespace/$crd/$cr.yaml
func recordCRs(namespaceDir string, namespace string) error {
	crds, err := kubeList(namespace, "crd")
	if err != nil {
		return err
	}

	// record all unique CRs floating about
	for _, crd := range crds {
		// consider all installed CRDs that are kgateway-managed
		if !strings.Contains(crd, "kgateway.dev") && !strings.Contains(crd, "networking.k8s.io") {
			continue
		}

		// if there are any existing CRs corresponding to this CRD
		crs, err := kubeList(namespace, crd)
		if err != nil {
			return err
		}
		if len(crs) == 0 {
			continue
		}
		crdDir := filepath.Join(namespaceDir, crd)
		if err := os.MkdirAll(crdDir, os.ModePerm); err != nil {
			return err
		}

		// we record each one in its own .yaml representation
		for _, cr := range crs {
			f := fileAtPath(filepath.Join(crdDir, cr+".yaml"))
			errF := fileAtPath(filepath.Join(crdDir, cr+"-error.log"))

			crDetails, errOutput, err := kubeGet(namespace, crd, cr)

			if crDetails != "" {
				f.WriteString(crDetails)
				f.Close()
			}
			if errOutput != "" {
				errF.WriteString(errOutput)
				errF.Close()
			}

			if err != nil {
				fmt.Printf("error getting cr: %s\n", err)
			}
		}
	}

	return nil
}

// kubeLogs runs $(kubectl -n $namespace logs $pod --all-containers) and returns the string result
func kubeLogs(namespace string, pod string) (string, string, error) {
	args := []string{"-n", namespace, "logs", pod, "--all-containers"}
	return kubeExecute(args)
}

// kubeGet runs $(kubectl -n $namespace get $kubeType $name -oyaml) and returns the string result
func kubeGet(namespace string, kubeType string, name string) (string, string, error) {
	args := []string{"-n", namespace, "get", kubeType, name, "-oyaml"}
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
