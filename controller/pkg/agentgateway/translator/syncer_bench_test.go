package translator_test

import (
	"bytes"
	"fmt"
	"log/slog"
	"maps"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"runtime/metrics"
	"strings"
	"sync/atomic"
	"testing"
	"text/template"
	"time"

	"github.com/Masterminds/sprig/v3"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	istiolog "istio.io/istio/pkg/log"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/test"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	agwv1alpha1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/krtxds"
)

// BenchmarkSyncer benchmarks the full syncer stack against scenarios built from
// pilot-load style templates in testdata/bench.
//
// A scenario is a list of configs; each renders a template Replicas times in each of its
// Namespaces, naming instances <Name>-<index>. Once the scenario is synced, every config whose template is marked
// `#refresh=true` gets its own benchmark. Each iteration refreshes one instance, the way a
// pilot-load refresh does (.Rand flips and .RandNumber changes), then waits for the controller
// to go idle.
//
// Templates get .Name, .Namespace, .Index (unique across namespaces, e.g. for IPs), .Rand,
// .RandNumber, the config's Vars, and the sprig functions.
//
// Reported metrics:
//   - ns/op, B/op, allocs/op: the full cost of one refresh, until every collection is idle.
//   - xds-changes/op: XDS resources changed per refresh. 0 means the refresh did no real work.
//   - retained-bytes: heap held by the synced scenario, inputs included.
//
// Example:
//
//	go test ./pkg/agentgateway/translator/ -run '^$' -bench 'BenchmarkSyncer/httproutes/refresh=httproute' -benchmem
func BenchmarkSyncer(b *testing.B) {
	// This does some lazy-loading, so load it upfront out of the benchmark
	_ = kube.NewFakeClient()
	gateways, gatewayConfigs := gatewayConfigs()

	// Each app has a Service, a Pod, and an HTTPRoute attached directly to every gateway.
	b.Run("httproutes", func(b *testing.B) {
		const namespaces = 50
		const replicas = 100
		runScenario(b, append(slices.Clone(gatewayConfigs),
			benchConfig{Template: "app", Namespace: "mesh", Namespaces: namespaces, Replicas: replicas},
			benchConfig{
				Template: "httproute", Name: "app", Namespace: "mesh", Namespaces: namespaces, Replicas: replicas,
				Vars: map[string]any{"gateways": gateways},
			},
		))
	})

	// Each app has a Service, a Pod, its own ListenerSet on every gateway, and many HTTPRoutes
	// attached to those ListenerSets. A TLS certificate is shared per namespace.
	b.Run("listenersets", func(b *testing.B) {
		const namespaces = 10
		const replicas = 100
		runScenario(b, append(slices.Clone(gatewayConfigs),
			benchConfig{Template: "tls-secret", Namespace: "mesh", Namespaces: namespaces, Vars: map[string]any{"Name": "ns-cert"}},
			benchConfig{Template: "app", Namespace: "mesh", Namespaces: namespaces, Replicas: replicas},
			benchConfig{
				Template: "route", Name: "app", Namespace: "mesh", Namespaces: namespaces, Replicas: replicas,
				Vars: map[string]any{"gateways": gateways, "routes": 16},
			},
			benchConfig{
				Template: "listenerset", Name: "app", Namespace: "mesh", Namespaces: namespaces, Replicas: replicas,
				Vars: map[string]any{"gateways": gateways},
			},
		))
	})
}

// gatewayConfigs returns the node pods run on, our Gateway plus 5 Gateways owned by another controller, and the
// "<namespace>/<name>" of each for routes to attach to.
func gatewayConfigs() ([]string, []benchConfig) {
	gateways := []string{"gateways/agentgateway-0"}
	for i := range 5 {
		gateways = append(gateways, fmt.Sprintf("gateways/other-%d", i))
	}
	return gateways, []benchConfig{
		{Template: "node"},
		{Template: "gatewayclass", Vars: map[string]any{"className": "agentgateway", "controller": "agentgateway.dev/agentgateway"}},
		{Template: "gatewayclass", Name: "other-gatewayclass", Vars: map[string]any{"className": "other", "controller": "example.com/other"}},
		{Template: "gateway", Name: "agentgateway", Namespace: "gateways", Vars: map[string]any{"className": "agentgateway"}},
		{Template: "tls-secret", Name: "agentgateway", Namespace: "gateways"},
		{Template: "gateway", Name: "other", Namespace: "gateways", Replicas: 5, Vars: map[string]any{"className": "other"}},
	}
}

type benchConfig struct {
	// Template is the file name in testdata/bench, without extension.
	Template string
	// Name prefixes instance names (<Name>-<index>). Defaults to Template. Configs sharing a Name
	// render matching names, e.g. an HTTPRoute pointing at the Service of the same app.
	Name string
	// Namespace is set on rendered objects that do not set one. Leave empty for cluster scoped objects.
	Namespace string
	// Namespaces, if set, spreads instances over that many namespaces named <Namespace>-<n>.
	Namespaces int
	// Replicas is the number of instances per namespace. Defaults to 1.
	Replicas int
	// Vars are extra template inputs. A "Name" var replaces the generated instance name.
	Vars map[string]any
}

func runScenario(b *testing.B, configs []benchConfig) {
	templates := loadBenchTemplates(b)
	perTemplate := map[string]int{}
	for _, cfg := range configs {
		perTemplate[cfg.Template]++
	}
	for i, cfg := range configs {
		// Name benchmarks after the template, adding the config name when a template is used twice.
		refresh := cfg.Template
		if perTemplate[cfg.Template] > 1 {
			refresh += "-" + cfg.name()
		}
		if strings.Contains(templates[cfg.Template].Root.String(), "#refresh=true") {
			b.Run("refresh="+refresh, func(b *testing.B) {
				benchRefresh(b, templates, configs, i)
			})
		}
	}
}

func benchRefresh(b *testing.B, templates map[string]*template.Template, configs []benchConfig, refreshed int) {
	logging.Reset(slog.LevelWarn)
	for _, scope := range istiolog.Scopes() {
		scope.SetOutputLevel(istiolog.WarnLevel)
	}
	stop := test.NewStop(b)
	before := liveHeapBytes()

	// Render the initial state. The refreshed config's object kinds go into mutable collections so
	// they can be updated; everything else is static.
	namespaces := map[string]bool{}
	var initial [][][]renderedObject // [config][instance][object]
	for _, cfg := range configs {
		var instances [][]renderedObject
		for _, inst := range cfg.instances() {
			instances = append(instances, renderInstance(b, templates[cfg.Template], cfg, inst, false))
			if inst.namespace != "" {
				namespaces[inst.namespace] = true
			}
		}
		initial = append(initial, instances)
	}
	mutableTypes := map[reflect.Type]bool{}
	for _, inst := range initial[refreshed] {
		for _, o := range inst {
			mutableTypes[reflect.TypeOf(o.obj)] = true
		}
	}
	staticObjs := []any{}
	for ns := range namespaces {
		staticObjs = append(staticObjs, &corev1.Namespace{ObjectMeta: metav1.ObjectMeta{Name: ns}})
	}
	mutableObjs := map[reflect.Type][]any{}
	for _, instances := range initial {
		for _, inst := range instances {
			for _, o := range inst {
				t := reflect.TypeOf(o.obj)
				if mutableTypes[t] {
					mutableObjs[t] = append(mutableObjs[t], o.obj)
				} else {
					staticObjs = append(staticObjs, o.obj)
				}
			}
		}
	}

	col := testutils.BuildMockCollection(b, staticObjs)
	updaters := map[reflect.Type]func(any){}
	for t, objs := range mutableObjs {
		in := slices.FindFunc(mutableInputs, func(in mutableInput) bool { return in.typ == t })
		if in == nil {
			b.Fatalf("refreshing %v is not supported; add it to mutableInputs", t)
		}
		updaters[t] = in.bind(col, objs, stop)
	}
	col.SetupIndexes()
	_, s := testutils.Syncer(b, plugins.PolicyCtx{Collections: col})
	ds := krtxds.NewDiscoveryServer(nil, nil, s.Registrations...)
	for _, c := range ds.Collections {
		c.Col.WaitUntilSynced(stop)
	}
	waitIdle()
	retained := liveHeapBytes() - before

	// Pre-render the refreshed state of every instance, so the loop does no rendering or decoding.
	// Only objects that actually differ are re-applied, like a server side apply would.
	cfg := configs[refreshed]
	type refresh struct{ apply, revert []any }
	var refreshes []refresh
	for i, inst := range cfg.instances() {
		base := initial[refreshed][i]
		var r refresh
		for j, o := range renderInstance(b, templates[cfg.Template], cfg, inst, true) {
			if o.yaml != base[j].yaml {
				r.apply = append(r.apply, o.obj)
				r.revert = append(r.revert, base[j].obj)
			}
		}
		if len(r.apply) == 0 {
			b.Fatalf("refreshing %s renders identical objects; the template must use .Rand or .RandNumber", cfg.Template)
		}
		refreshes = append(refreshes, r)
	}

	var xdsChanges atomic.Int64
	for _, c := range ds.Collections {
		c.Col.RegisterBatch(func(events []krt.Event[krtxds.DiscoveryResource]) {
			xdsChanges.Add(int64(len(events)))
		}, false)
	}

	// Refresh instances round-robin. Odd passes revert to the initial state, so every refresh is a
	// real change.
	for i := 0; b.Loop(); i++ {
		r := refreshes[i%len(refreshes)]
		objs := r.apply
		if (i/len(refreshes))%2 == 1 {
			objs = r.revert
		}
		for _, o := range objs {
			updaters[reflect.TypeOf(o)](o)
		}
		waitIdle()
	}
	b.ReportMetric(float64(xdsChanges.Load())/float64(b.N), "xds-changes/op")
	b.ReportMetric(float64(retained), "retained-bytes")
}

type renderedObject struct {
	yaml string
	obj  controllers.Object
}

func (cfg benchConfig) name() string {
	if cfg.Name != "" {
		return cfg.Name
	}
	if name, ok := cfg.Vars["Name"].(string); ok {
		return name
	}
	return cfg.Template
}

type instance struct {
	namespace string
	// index is the position within the namespace; global is unique across all namespaces.
	index, global int
}

func (cfg benchConfig) instances() []instance {
	var res []instance
	for n := range max(cfg.Namespaces, 1) {
		ns := cfg.Namespace
		if cfg.Namespaces > 0 {
			ns = fmt.Sprintf("%s-%d", cfg.Namespace, n)
		}
		for i := range max(cfg.Replicas, 1) {
			res = append(res, instance{namespace: ns, index: i, global: len(res)})
		}
	}
	return res
}

func renderInstance(t testing.TB, tmpl *template.Template, cfg benchConfig, inst instance, refreshed bool) []renderedObject {
	data := maps.Clone(cfg.Vars)
	if data == nil {
		data = map[string]any{}
	}
	if _, ok := data["Name"]; !ok {
		data["Name"] = fmt.Sprintf("%s-%d", cfg.name(), inst.index)
	}
	data["Namespace"] = inst.namespace
	data["Index"] = inst.global
	data["Rand"] = refreshed
	// Deterministic stand-in for pilot-load's random number; always differs after a refresh.
	randNumber := 1 + inst.global*7919%5000
	if refreshed {
		randNumber += 5000
	}
	data["RandNumber"] = randNumber

	var buf bytes.Buffer
	if err := tmpl.Execute(&buf, data); err != nil {
		t.Fatalf("render %s: %v", cfg.Template, err)
	}
	var res []renderedObject
	for _, doc := range strings.Split(buf.String(), "\n---\n") {
		if isEmptyYAML(doc) {
			continue
		}
		o, _, err := kube.IstioCodec.UniversalDeserializer().Decode([]byte(doc), nil, nil)
		if err != nil {
			t.Fatalf("decode %s: %v\n%s", cfg.Template, err, doc)
		}
		obj := o.(controllers.Object)
		if obj.GetNamespace() == "" && inst.namespace != "" {
			obj.SetNamespace(inst.namespace)
		}
		// The API server merges stringData into data; do the same.
		if s, ok := obj.(*corev1.Secret); ok && len(s.StringData) > 0 {
			if s.Data == nil {
				s.Data = map[string][]byte{}
			}
			for k, v := range s.StringData {
				s.Data[k] = []byte(v)
			}
			s.StringData = nil
		}
		res = append(res, renderedObject{yaml: doc, obj: obj})
	}
	return res
}

func isEmptyYAML(doc string) bool {
	for _, line := range strings.Split(doc, "\n") {
		line = strings.TrimSpace(line)
		if line != "" && !strings.HasPrefix(line, "#") {
			return false
		}
	}
	return true
}

func loadBenchTemplates(t testing.TB) map[string]*template.Template {
	files, err := filepath.Glob("testdata/bench/*.yaml")
	if err != nil {
		t.Fatal(err)
	}
	res := map[string]*template.Template{}
	for _, f := range files {
		by, err := os.ReadFile(f)
		if err != nil {
			t.Fatal(err)
		}
		name := strings.TrimSuffix(filepath.Base(f), ".yaml")
		tmpl, err := template.New(name).Funcs(sprig.TxtFuncMap()).Parse(string(by))
		if err != nil {
			t.Fatalf("parse %s: %v", f, err)
		}
		res[name] = tmpl
	}
	return res
}

// mutableInputs are the input collections a refresh can update.
var mutableInputs = []mutableInput{
	input(func(c *plugins.AgwCollections) *krt.Collection[*corev1.Service] { return &c.Services }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*corev1.Pod] { return &c.Pods }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*corev1.Secret] { return &c.Secrets }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*corev1.ConfigMap] { return &c.ConfigMaps }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*gwv1.Gateway] { return &c.Gateways }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*gwv1.ListenerSet] { return &c.ListenerSets }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*gwv1.HTTPRoute] { return &c.HTTPRoutes }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*gwv1.GRPCRoute] { return &c.GRPCRoutes }),
	input(func(c *plugins.AgwCollections) *krt.Collection[*agwv1alpha1.AgentgatewayPolicy] {
		return &c.AgentgatewayPolicies
	}),
	input(func(c *plugins.AgwCollections) *krt.Collection[*agwv1alpha1.AgentgatewayBackend] { return &c.Backends }),
}

type mutableInput struct {
	typ reflect.Type
	// bind replaces the collection with a mutable one holding objs, and returns its update function.
	bind func(col *plugins.AgwCollections, objs []any, stop <-chan struct{}) func(any)
}

func input[T controllers.Object](field func(*plugins.AgwCollections) *krt.Collection[T]) mutableInput {
	return mutableInput{
		typ: reflect.TypeFor[T](),
		bind: func(col *plugins.AgwCollections, objs []any, stop <-chan struct{}) func(any) {
			c := krt.NewMutableCollection(nil, slices.Map(objs, func(o any) T { return o.(T) }), krt.WithStop(stop))
			*field(col) = c.AsCollection()
			return func(o any) { c.UpdateObject(o.(T)) }
		},
	}
}

// waitIdle blocks until no other goroutine is running or runnable, which (absent timers) means
// every krt queue has drained.
func waitIdle() {
	samples := []metrics.Sample{
		{Name: "/sched/goroutines/running:goroutines"},
		{Name: "/sched/goroutines/runnable:goroutines"},
	}
	// While other goroutines are busy, spin for up to 1ms then back off, so long recomputes do not
	// burn a core polling. Idleness is confirmed without sleeping: a sleep in an idle process takes far
	// longer than requested and would put a floor on every measurement.
	start := time.Now()
	backoff := time.Duration(0)
	for idle := 0; idle < 3; {
		runtime.Gosched()
		metrics.Read(samples)
		if samples[0].Value.Uint64() <= 1 && samples[1].Value.Uint64() == 0 {
			idle++
			continue
		}
		idle = 0
		if time.Since(start) > time.Millisecond {
			backoff = min(max(backoff*2, 10*time.Microsecond), time.Millisecond)
			time.Sleep(backoff)
		}
	}
}

// liveHeapBytes returns the heap in use after a full garbage collection.
func liveHeapBytes() int64 {
	runtime.GC()
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	return int64(m.HeapAlloc)
}
