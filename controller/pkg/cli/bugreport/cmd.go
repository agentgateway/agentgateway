package bugreport

import (
	"archive/tar"
	"cmp"
	"compress/gzip"
	"context"
	"errors"
	"fmt"
	"os"
	"path"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/spf13/cobra"
	"golang.org/x/sync/errgroup"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/util/sets"
	"k8s.io/apimachinery/pkg/version"
	"k8s.io/client-go/tools/pager"
	"sigs.k8s.io/yaml"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	pkgversion "github.com/agentgateway/agentgateway/controller/pkg/version"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const (
	controllerSelector    = "agentgateway=agentgateway"
	proxyStatsPort        = "15020"
	controllerMetricsPort = "9092"
	maxLogBytes           = 10 << 20
	concurrency           = 8
	// base64("redacted")
	redacted = "cmVkYWN0ZWQ="
)

var (
	gatewayAPIGroups = sets.New("gateway.networking.k8s.io", "gateway.networking.x-k8s.io", "inference.networking.k8s.io")
	podsGVR          = corev1.SchemeGroupVersion.WithResource("pods")
	// Listed in every namespace that holds a captured object or the controller.
	namespaceGVRs = []schema.GroupVersionResource{
		corev1.SchemeGroupVersion.WithResource("services"),
		corev1.SchemeGroupVersion.WithResource("configmaps"),
		corev1.SchemeGroupVersion.WithResource("secrets"),
		corev1.SchemeGroupVersion.WithResource("events"),
	}
)

type flags struct {
	namespaces []string
	since      time.Duration
	tail       int
	configDump bool
	timeout    time.Duration
	output     string
}

func Command() flag.Command {
	f := &flags{
		since:   24 * time.Hour,
		tail:    5000,
		timeout: 5 * time.Minute,
		output:  "bug-report.tar.gz",
	}
	return flag.Command{
		Use:   "bug-report",
		Short: "Collect agentgateway resources, logs, and metrics into an archive",
		Long: `Collect agentgateway resources, logs, and metrics into an archive for bug reports.

Captured resources are all Gateway API resources, and all resources with a category containing
"agentgateway". Services, ConfigMaps, Secrets, and Events are captured from every namespace holding one of
those resources, or the controller. Secret values are redacted.

Logs and metrics are captured from agentgateway proxy pods and all pods in the controller namespace.`,
		Example: `agctl bug-report
agctl bug-report -n team-a -n team-b --since 1h
agctl bug-report --config-dump -o /tmp/report.tar.gz`,
		Args: cobra.NoArgs,
		AddFlags: func(cmd *cobra.Command) {
			flag.RegisterShort(cmd.Flags(), &f.namespaces, "namespace", "n", "Only capture namespaced resources and proxies in these namespaces. The controller namespace is always captured.")
			flag.Register(cmd.Flags(), &f.since, "since", "Only capture logs newer than this duration")
			flag.Register(cmd.Flags(), &f.tail, "tail", "Maximum log lines per container; 0 for no limit")
			flag.Register(cmd.Flags(), &f.configDump, "config-dump", "Capture /config_dump from each proxy")
			flag.Register(cmd.Flags(), &f.timeout, "timeout", "Deadline for the whole collection; anything collected before it is still written")
			flag.RegisterShort(cmd.Flags(), &f.output, "output", "o", "Output archive")
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			return run(cmd, f)
		},
	}
}

type collector struct {
	client kubeutil.CLIClient
	f      *flags
	now    time.Time

	mu      sync.Mutex
	tw      *tar.Writer
	objects map[string]*unstructured.Unstructured
	errs    []error
	// Names of GatewayClasses owned by agentgateway; proxy Pods carry one in wellknown.GatewayClassNameLabel.
	proxyClasses sets.Set[string]
}

func run(cmd *cobra.Command, f *flags) error {
	client, err := kubeutil.NewCLIClient()
	if err != nil {
		return err
	}
	out, err := os.Create(f.output)
	if err != nil {
		return err
	}
	defer out.Close()
	gz := gzip.NewWriter(out)
	c := &collector{
		client:  client,
		f:       f,
		now:     time.Now(),
		tw:      tar.NewWriter(gz),
		objects: map[string]*unstructured.Unstructured{},
	}

	ctx, cancel := context.WithTimeout(cmd.Context(), f.timeout)
	defer cancel()
	c.collect(ctx)

	c.writeResources()
	if len(c.errs) > 0 {
		c.writeFile("errors.txt", []byte(errors.Join(c.errs...).Error()+"\n"))
	}
	if err := c.tw.Close(); err != nil {
		return err
	}
	if err := gz.Close(); err != nil {
		return err
	}
	fmt.Fprintf(cmd.OutOrStdout(), "Wrote %s (%d resources, %d errors)\n", f.output, len(c.objects), len(c.errs))
	return nil
}

func (c *collector) collect(ctx context.Context) {
	sv, err := c.client.Kube().Discovery().ServerVersion()
	c.addErr(err, "get server version")
	versions, _ := yaml.Marshal(struct {
		Agctl      *pkgversion.VersionInfo `json:"agctl"`
		Kubernetes *version.Info           `json:"kubernetes,omitempty"`
	}{pkgversion.Info(), sv})
	c.writeFile("version.yaml", versions)

	// Partial results are returned when some API groups are unavailable.
	resources, err := c.client.Kube().Discovery().ServerPreferredResources()
	c.addErr(err, "discover resources")

	g := c.group()
	for _, list := range resources {
		gv, err := schema.ParseGroupVersion(list.GroupVersion)
		if err != nil {
			continue
		}
		for _, r := range list.APIResources {
			if strings.Contains(r.Name, "/") || !slices.Contains(r.Verbs, "list") {
				continue
			}
			ours := slices.ContainsFunc(r.Categories, func(c string) bool { return strings.Contains(c, "agentgateway") })
			if !ours && !gatewayAPIGroups.Has(gv.Group) {
				continue
			}
			for _, ns := range c.listNamespaces(r.Namespaced) {
				g.Go(func() error {
					c.addObjects(c.list(ctx, gv.WithResource(r.Name), ns, ""))
					return nil
				})
			}
		}
	}
	var controllerPods []*unstructured.Unstructured
	g.Go(func() error {
		controllerPods = c.list(ctx, podsGVR, "", controllerSelector)
		return nil
	})
	_ = g.Wait()

	c.proxyClasses = sets.New[string]()
	for _, o := range c.objects {
		controller, _, _ := unstructured.NestedString(o.Object, "spec", "controllerName")
		if o.GetKind() == "GatewayClass" && controller == wellknown.DefaultAgwControllerName {
			c.proxyClasses.Insert(o.GetName())
		}
	}
	if c.proxyClasses.Len() > 0 {
		selector := fmt.Sprintf("%s in (%s)", wellknown.GatewayClassNameLabel, strings.Join(sets.List(c.proxyClasses), ","))
		g = c.group()
		for _, ns := range c.listNamespaces(true) {
			g.Go(func() error {
				c.addObjects(c.list(ctx, podsGVR, ns, selector))
				return nil
			})
		}
		_ = g.Wait()
	}

	controllerNamespaces := sets.New[string]()
	for _, p := range controllerPods {
		controllerNamespaces.Insert(p.GetNamespace())
	}
	namespaces := controllerNamespaces.Clone()
	for _, o := range c.objects {
		if o.GetNamespace() != "" {
			namespaces.Insert(o.GetNamespace())
		}
	}

	g = c.group()
	for _, ns := range sets.List(namespaces) {
		gvrs := namespaceGVRs
		if controllerNamespaces.Has(ns) {
			gvrs = append(slices.Clone(gvrs), podsGVR)
		}
		for _, gvr := range gvrs {
			g.Go(func() error {
				c.addObjects(c.list(ctx, gvr, ns, ""))
				return nil
			})
		}
	}
	_ = g.Wait()

	g = c.group()
	for _, o := range c.objects {
		if o.GetAPIVersion() != "v1" || o.GetKind() != "Pod" {
			continue
		}
		pod := &corev1.Pod{}
		if err := runtime.DefaultUnstructuredConverter.FromUnstructured(o.Object, pod); err != nil {
			c.addErr(err, "convert pod %s/%s", o.GetNamespace(), o.GetName())
			continue
		}
		g.Go(func() error {
			c.collectPod(ctx, pod)
			return nil
		})
	}
	_ = g.Wait()
}

func (c *collector) collectPod(ctx context.Context, pod *corev1.Pod) {
	dir := path.Join(pod.Namespace, pod.Name)
	pods := c.client.Kube().CoreV1().Pods(pod.Namespace)
	for _, st := range pod.Status.ContainerStatuses {
		for _, previous := range []bool{false, true} {
			if previous && st.RestartCount == 0 {
				continue
			}
			opts := &corev1.PodLogOptions{
				Container:  st.Name,
				Previous:   previous,
				LimitBytes: new(int64(maxLogBytes)),
			}
			if c.f.tail > 0 {
				opts.TailLines = new(int64(c.f.tail))
			}
			if c.f.since > 0 {
				opts.SinceSeconds = new(int64(c.f.since.Seconds()))
			}
			name := st.Name + ".log"
			if previous {
				name = st.Name + ".previous.log"
			}
			logs, err := pods.GetLogs(pod.Name, opts).DoRaw(ctx)
			if c.addErr(err, "get logs %s/%s", dir, name) {
				c.writeFile(path.Join("logs", dir, name), logs)
			}
		}
	}

	if pod.Status.Phase != corev1.PodRunning {
		return
	}
	proxy := c.proxyClasses.Has(pod.Labels[wellknown.GatewayClassNameLabel])
	metricsPort := ""
	switch {
	case proxy:
		metricsPort = proxyStatsPort
	case pod.Labels["agentgateway"] == "agentgateway":
		metricsPort = controllerMetricsPort
	}
	if metricsPort != "" {
		metrics, err := pods.ProxyGet("http", pod.Name, metricsPort, "metrics", nil).DoRaw(ctx)
		if c.addErr(err, "get metrics %s", dir) {
			c.writeFile(path.Join("metrics", dir+".txt"), metrics)
		}
	}
	if proxy && c.f.configDump {
		dump, err := c.client.AgentgatewayRequest(ctx, pod.Name, pod.Namespace, "GET", "config_dump", wellknown.ProxyAdminPort)
		if c.addErr(err, "get config dump %s", dir) {
			c.writeFile(path.Join("config_dump", dir+".json"), dump)
		}
	}
}

// list pages through a resource. ResourceVersion is left unset so the API server serves a consistent read
// from its watch cache; it must never be "0" with a limit, as some versions then ignore the limit.
func (c *collector) list(ctx context.Context, gvr schema.GroupVersionResource, namespace, selector string) []*unstructured.Unstructured {
	p := pager.New(func(ctx context.Context, opts metav1.ListOptions) (runtime.Object, error) {
		return c.client.Dynamic().Resource(gvr).Namespace(namespace).List(ctx, opts)
	})
	var out []*unstructured.Unstructured
	err := p.EachListItem(ctx, metav1.ListOptions{LabelSelector: selector}, func(obj runtime.Object) error {
		out = append(out, obj.(*unstructured.Unstructured))
		return nil
	})
	c.addErr(err, "list %s in namespace %q", gvr.GroupResource(), namespace)
	return out
}

func (c *collector) listNamespaces(namespaced bool) []string {
	if !namespaced || len(c.f.namespaces) == 0 {
		return []string{metav1.NamespaceAll}
	}
	return c.f.namespaces
}

func (c *collector) group() *errgroup.Group {
	g := &errgroup.Group{}
	g.SetLimit(concurrency)
	return g
}

func (c *collector) addObjects(objs []*unstructured.Unstructured) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for _, o := range objs {
		c.objects[o.GetAPIVersion()+"/"+o.GetKind()+"/"+o.GetNamespace()+"/"+o.GetName()] = o
	}
}

// addErr records err, if any, and reports whether the operation succeeded.
func (c *collector) addErr(err error, format string, args ...any) bool {
	if err == nil {
		return true
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	c.errs = append(c.errs, fmt.Errorf("%s: %w", fmt.Sprintf(format, args...), err))
	return false
}

func (c *collector) writeFile(name string, data []byte) {
	c.mu.Lock()
	defer c.mu.Unlock()
	err := c.tw.WriteHeader(&tar.Header{
		Name:    path.Join("bug-report", name),
		Mode:    0o644,
		Size:    int64(len(data)),
		ModTime: c.now,
	})
	if err == nil {
		_, err = c.tw.Write(data)
	}
	if err != nil {
		c.errs = append(c.errs, fmt.Errorf("write %s: %w", name, err))
	}
}

func (c *collector) writeResources() {
	objs := make([]*unstructured.Unstructured, 0, len(c.objects))
	for _, o := range c.objects {
		objs = append(objs, o)
	}
	slices.SortFunc(objs, func(a, b *unstructured.Unstructured) int {
		return cmp.Or(
			cmp.Compare(a.GetAPIVersion(), b.GetAPIVersion()),
			cmp.Compare(a.GetKind(), b.GetKind()),
			cmp.Compare(a.GetNamespace(), b.GetNamespace()),
			cmp.Compare(a.GetName(), b.GetName()),
		)
	})
	var sb strings.Builder
	for _, o := range objs {
		o.SetManagedFields(nil)
		annotations := o.GetAnnotations()
		delete(annotations, corev1.LastAppliedConfigAnnotation)
		o.SetAnnotations(annotations)
		if o.GetAPIVersion() == "v1" && o.GetKind() == "Secret" {
			if data, ok := o.Object["data"].(map[string]any); ok {
				for k := range data {
					data[k] = redacted
				}
			}
		}
		b, err := yaml.Marshal(o.Object)
		if !c.addErr(err, "marshal %s %s/%s", o.GetKind(), o.GetNamespace(), o.GetName()) {
			continue
		}
		sb.WriteString("---\n")
		sb.Write(b)
	}
	c.writeFile("resources.yaml", []byte(sb.String()))
}
