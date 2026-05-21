package helpers

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"golang.org/x/sync/errgroup"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/kubernetes"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

type resourceDumpSpec struct {
	Name       string
	GVK        schema.GroupVersionKind
	Namespaced bool
}

// StandardAgentgatewayDumpOnFail creates a dump of the kubernetes state and certain envoy data from
// the admin interface when a test fails.
// Look at `KubeDumpOnFail` && `EnvoyDumpOnFail` for more details
// nolint: forbidigo // lint is meant for controllers not e2e helpers
func StandardAgentgatewayDumpOnFail(outLog io.Writer, kubeClient client.Client, clientset kubernetes.Interface, outDir string, namespaces []string) {
	if testutils.ShouldSkipDump() {
		return
	}
	fmt.Printf("Test failed. Dumping state from %s...\n", strings.Join(namespaces, ", "))

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	// only wipe at the start of the dump
	wipeOutDir(outDir)

	KubeDumpOnFail(ctx, kubeClient, clientset, outLog, outDir, namespaces)

	fmt.Printf("Test failed. Logs and cluster state are available in %s\n", outDir)
}

// KubeDumpOnFail creates a small dump of the kubernetes state when a test fails.
// This is useful for debugging test failures.
// The dump includes:
// - docker state
// - process state
// - kubernetes state
// - logs from all pods in the given namespaces
// - yaml representations of all agentgateway CRs in the given namespaces
// nolint: forbidigo // lint is meant for controllers not e2e helpers
func KubeDumpOnFail(ctx context.Context, kubeClient client.Client, clientset kubernetes.Interface, outLog io.Writer, outDir string,
	namespaces []string,
) {
	t0 := time.Now()
	setupOutDir(outDir)

	recordKubeState(ctx, kubeClient, fileAtPath(filepath.Join(outDir, "kube-state.log")), namespaces)

	recordKubeDump(ctx, clientset, outDir, namespaces...)

	fmt.Printf("Finished dumping kubernetes state (%v)\n", time.Since(t0))
}

// nolint: forbidigo // lint is meant for controllers not e2e helpers
func recordKubeState(ctx context.Context, kubeClient client.Client, f *os.File, namespaces []string) {
	defer f.Close()

	resourcesToGet := []resourceDumpSpec{
		// Kubernetes resources
		{Name: "secrets", GVK: corev1.SchemeGroupVersion.WithKind("Secret"), Namespaced: true},
		{Name: "services", GVK: corev1.SchemeGroupVersion.WithKind("Service"), Namespaced: true},
		{Name: "pods", GVK: corev1.SchemeGroupVersion.WithKind("Pod"), Namespaced: true},
		{Name: "deployments", GVK: schema.GroupVersionKind{Group: "apps", Version: "v1", Kind: "Deployment"}, Namespaced: true},
		{Name: "configmaps", GVK: corev1.SchemeGroupVersion.WithKind("ConfigMap"), Namespaced: true},
		{Name: "events", GVK: corev1.SchemeGroupVersion.WithKind("Event"), Namespaced: true},
		// Agentgateway
		{Name: "agentgatewaybackends.agentgateway.dev", GVK: schema.GroupVersionKind{Group: "agentgateway.dev", Version: "v1alpha1", Kind: "AgentgatewayBackend"}, Namespaced: true},
		{Name: "agentgatewayparameters.agentgateway.dev", GVK: schema.GroupVersionKind{Group: "agentgateway.dev", Version: "v1alpha1", Kind: "AgentgatewayParameters"}, Namespaced: true},
		{Name: "agentgatewaypolicies.agentgateway.dev", GVK: schema.GroupVersionKind{Group: "agentgateway.dev", Version: "v1alpha1", Kind: "AgentgatewayPolicy"}, Namespaced: true},
		// Kube GW API resources
		{Name: "backendtlspolicies.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1", Kind: "BackendTLSPolicy"}, Namespaced: true},
		{Name: "gatewayclasses.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1", Kind: "GatewayClass"}},
		{Name: "gateways.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1", Kind: "Gateway"}, Namespaced: true},
		{Name: "grpcroutes.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1", Kind: "GRPCRoute"}, Namespaced: true},
		{Name: "httproutes.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1", Kind: "HTTPRoute"}, Namespaced: true},
		{Name: "referencegrants.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1beta1", Kind: "ReferenceGrant"}, Namespaced: true},
		{Name: "tcproutes.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1alpha2", Kind: "TCPRoute"}, Namespaced: true},
		{Name: "tlsroutes.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1", Kind: "TLSRoute"}, Namespaced: true},
		{Name: "udproutes.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1alpha2", Kind: "UDPRoute"}, Namespaced: true},
		{Name: "listenersets.gateway.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.k8s.io", Version: "v1alpha1", Kind: "ListenerSet"}, Namespaced: true},
		{Name: "xbackendtrafficpolicies.gateway.networking.x-k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.x-k8s.io", Version: "v1alpha1", Kind: "XBackendTrafficPolicy"}, Namespaced: true},
		{Name: "xmeshes.gateway.networking.x-k8s.io", GVK: schema.GroupVersionKind{Group: "gateway.networking.x-k8s.io", Version: "v1alpha1", Kind: "XMesh"}, Namespaced: true},
		// GIE
		{Name: "inferencepools.inference.networking.k8s.io", GVK: schema.GroupVersionKind{Group: "inference.networking.k8s.io", Version: "v1alpha2", Kind: "InferencePool"}, Namespaced: true},
	}

	f.WriteString("*** Kube resources ***\n")
	for _, resource := range resourcesToGet {
		recordResourceState(ctx, kubeClient, f, resource, namespaces)
	}
}

// nolint: forbidigo // lint is meant for controllers not e2e helpers
func recordResourceState(ctx context.Context, kubeClient client.Client, f *os.File, spec resourceDumpSpec, namespaces []string) {
	f.WriteString("\n*** " + spec.Name + " ***\n")
	if !spec.Namespaced {
		recordResourceList(ctx, kubeClient, f, spec, "")
		return
	}
	for _, ns := range namespaces {
		recordResourceList(ctx, kubeClient, f, spec, ns)
	}
}

// nolint: forbidigo // lint is meant for controllers not e2e helpers
func recordResourceList(ctx context.Context, kubeClient client.Client, f *os.File, spec resourceDumpSpec, namespace string) {
	list := &unstructured.UnstructuredList{}
	list.SetGroupVersionKind(schema.GroupVersionKind{
		Group:   spec.GVK.Group,
		Version: spec.GVK.Version,
		Kind:    spec.GVK.Kind + "List",
	})

	opts := []client.ListOption{}
	if namespace != "" {
		opts = append(opts, client.InNamespace(namespace))
	}
	if err := kubeClient.List(ctx, list, opts...); err != nil {
		f.WriteString(fmt.Sprintf("%s: unable to list: %v\n", namespaceOrCluster(namespace), err))
		return
	}
	if len(list.Items) == 0 {
		f.WriteString(fmt.Sprintf("%s: none\n", namespaceOrCluster(namespace)))
		return
	}
	for _, item := range list.Items {
		f.WriteString(fmt.Sprintf("%s/%s\n", namespaceOrCluster(item.GetNamespace()), item.GetName()))
	}
}

func namespaceOrCluster(namespace string) string {
	if namespace == "" {
		return "_cluster"
	}
	return namespace
}

func recordKubeDump(ctx context.Context, clientset kubernetes.Interface, outDir string, namespaces ...string) {
	g := errgroup.Group{}
	// for each namespace, create a namespace directory that contains...
	for _, ns := range namespaces {
		// ...a pod logs subdirectoy
		g.Go(func() error {
			return recordPods(ctx, clientset, filepath.Join(outDir, ns, "_pods"), ns)
		})
	}
	if err := g.Wait(); err != nil {
		fmt.Printf("error recording pod logs: %v, \n", err)
	}
}

// recordPods records logs from each pod to <output-dir>/$namespace/pods/$pod.log
func recordPods(ctx context.Context, clientset kubernetes.Interface, podDir, namespace string) error {
	pods, err := clientset.CoreV1().Pods(namespace).List(ctx, metav1.ListOptions{})
	if err != nil {
		return err
	}

	var errs []error
	var errsMu sync.Mutex

	if err := os.MkdirAll(podDir, os.ModePerm); err != nil {
		return err
	}
	g := errgroup.Group{}
	for _, pod := range pods.Items {
		g.Go(func() error {
			logs, err := podLogs(ctx, clientset, namespace, pod)
			if err != nil {
				errsMu.Lock()
				errs = append(errs, err)
				errsMu.Unlock()
			}
			// write any log output to the standard file
			if logs != "" {
				f := fileAtPath(filepath.Join(podDir, pod.Name+".log"))
				defer f.Close()
				f.WriteString(logs)
			}
			if err != nil {
				f := fileAtPath(filepath.Join(podDir, pod.Name+"-error.log"))
				defer f.Close()
				f.WriteString(err.Error())
			}

			return nil
		})
	}
	g.Wait()

	return errors.Join(errs...)
}

func podLogs(ctx context.Context, clientset kubernetes.Interface, namespace string, pod corev1.Pod) (string, error) {
	var logs strings.Builder
	var errs []error
	for _, container := range podLogContainers(pod) {
		logs.WriteString(fmt.Sprintf("==== %s/%s %s ====\n", namespace, pod.Name, container))
		stream, err := clientset.CoreV1().
			Pods(namespace).
			GetLogs(pod.Name, &corev1.PodLogOptions{Container: container}).
			Stream(ctx)
		if err != nil {
			errs = append(errs, fmt.Errorf("failed to stream logs for %s/%s container %s: %w", namespace, pod.Name, container, err))
			continue
		}
		if _, err := io.Copy(&logs, stream); err != nil {
			errs = append(errs, fmt.Errorf("failed to read logs for %s/%s container %s: %w", namespace, pod.Name, container, err))
		}
		if err := stream.Close(); err != nil {
			errs = append(errs, fmt.Errorf("failed to close log stream for %s/%s container %s: %w", namespace, pod.Name, container, err))
		}
		logs.WriteString("\n")
	}
	return logs.String(), errors.Join(errs...)
}

func podLogContainers(pod corev1.Pod) []string {
	containers := make([]string, 0, len(pod.Spec.InitContainers)+len(pod.Spec.Containers)+len(pod.Spec.EphemeralContainers))
	for _, container := range pod.Spec.InitContainers {
		containers = append(containers, container.Name)
	}
	for _, container := range pod.Spec.Containers {
		containers = append(containers, container.Name)
	}
	for _, container := range pod.Spec.EphemeralContainers {
		containers = append(containers, container.Name)
	}
	return containers
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
