package testutils

import (
	"context"
	"fmt"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"

	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pilot/test/util"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/kube/krt/krttest"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/file"
	corev1 "k8s.io/api/core/v1"
	discovery "k8s.io/api/discovery/v1"
	utilruntime "k8s.io/apimachinery/pkg/util/runtime"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"
	"sigs.k8s.io/yaml"

	apitests "github.com/kgateway-dev/kgateway/v2/api/tests"
	agwv1alpha1 "github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient/fake"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/status"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
	"github.com/kgateway-dev/kgateway/v2/pkg/schemes"
)

func CompareGolden(t test.Failer, content []byte, goldenFile string) {
	util.CompareContent(t, content, rewrite(goldenFile))
}

// rewrite rewrites a subname to having only printable characters and no white
// space.
func rewrite(s string) string {
	b := []byte{}
	for _, r := range s {
		switch {
		case isSpace(r):
			b = append(b, '_')
		case !strconv.IsPrint(r):
			s := strconv.QuoteRune(r)
			b = append(b, s[1:len(s)-1]...)
		default:
			b = append(b, string(r)...)
		}
	}
	return string(b)
}

func isSpace(r rune) bool {
	if r < 0x2000 {
		switch r {
		// Note: not the same as Unicode Z class.
		case '\t', '\n', '\v', '\f', '\r', ' ', 0x85, 0xA0, 0x1680:
			return true
		}
	} else {
		if r <= 0x200a {
			return true
		}
		switch r {
		case 0x2028, 0x2029, 0x202f, 0x205f, 0x3000:
			return true
		}
	}
	return false
}

func init() {
	// Add our types to Istio since we are using their library
	utilruntime.Must(schemes.AddToScheme(kube.IstioScheme))
}

func GetTestResource[T any](t *testing.T, collection krt.Collection[T]) T {
	t.Helper()
	l := collection.List()
	if len(l) != 1 {
		t.Fatalf("expected 1 resource, got %d", len(l))
	}
	return l[0]
}

var timestampRegex = regexp.MustCompile(`lastTransitionTime:.*`)

// RunForDirectory runs a set of tests against each file in a directory.
// The file should pass in the input YAMLs at the top of the file, and the expected outputs at the bottom of the file split by:
//
// ---
// # Output
// ... the output
//
// The output is generally created by running the test with `REFRESH_GOLDEN=true`.
func RunForDirectory[Status any, Output any](t *testing.T, base string, run func(t *testing.T, ctx plugins.PolicyCtx) (Status, []Output)) {
	val := apitests.NewKgatewayValidator(t)
	val.SkipMissing = true
	defaults, defaultsErr := file.AsString(filepath.Join(base, "_defaults.yaml"))
	for _, f := range file.ReadDirOrFail(t, base) {
		name := filepath.Base(f)
		if name == "_defaults.yaml" {
			continue
		}
		t.Run(name, func(t *testing.T) {
			data := file.AsStringOrFail(t, f)
			inputData := data
			idx := strings.Index(data, "---\n# Output")
			if idx != -1 {
				inputData = data[:idx-1]
			}
			assert.NoError(t, val.ValidateCustomResourceYAML(inputData, nil))
			mockObjs := []any{}
			if defaultsErr == nil {
				mockObjs = append(mockObjs, defaults)
			}
			mockObjs = append(mockObjs, inputData)
			ctx := BuildMockPolicyContext(t, mockObjs)
			st, objs := run(t, ctx)
			o, err := yaml.Marshal(testOutput[Status, Output]{Status: st, Output: objs})
			if err != nil {
				t.Fatalf("failed to marshal output: %v", err)
			}
			o = timestampRegex.ReplaceAll(o, []byte("lastTransitionTime: fake"))
			output := inputData + "\n---\n# Output\n" + string(o)
			if util.Refresh() {
				util.RefreshGoldenFile(t, []byte(output), f)
			} else {
				util.CompareBytes(t, []byte(output), []byte(data), name)
			}
		})
	}
}

type testOutput[Status any, Output any] struct {
	Status Status   `json:"status,omitempty"`
	Output []Output `json:"output"`
}

func Syncer(t *testing.T, ctx plugins.PolicyCtx, includeStatusKinds ...string) (*TestStatusQueue, *agentgatewaysyncer.Syncer) {
	fc := fake.NewClient(t)
	stop := test.NewStop(t)
	debugger := new(krt.DebugHandler)
	opts := krtutil.NewKrtOptions(stop, debugger)
	t.Cleanup(func() {
		if t.Failed() {
			b, _ := yaml.Marshal(debugger)
			t.Log(string(b))
		}
	})
	syncer := agentgatewaysyncer.NewAgwSyncer(
		context.Background(),
		wellknown.DefaultAgwControllerName,
		// Only used for NACK, so no need to do anything special here.
		fc,
		ctx.Collections,
		agwPluginFactory(test.NewContext(t), ctx.Collections),
		nil,
		opts,
		nil,
	)
	fc.RunAndWait(stop)
	sq := &TestStatusQueue{
		state:        map[status.Resource]any{},
		includeKinds: includeStatusKinds,
	}
	// Normally we don't care to block on status being written, but here we need to since we want to test output
	statusSynced := syncer.StatusCollections().SetQueue(sq)
	go syncer.Start(test.NewContext(t))
	kube.WaitForCacheSync("test", stop, syncer.HasSynced)
	for _, st := range statusSynced {
		st.WaitUntilSynced(stop)
	}
	sq.Dump()
	return sq, syncer
}

// agwPluginFactory is a factory function that returns the agent gateway plugins
// It is based on agwPluginFactory(cfg)(ctx, cfg.AgwCollections) in start.go
func agwPluginFactory(ctx context.Context, agwCollections *plugins.AgwCollections) plugins.AgwPlugin {
	agwPlugins := plugins.Plugins(agwCollections)
	mergedPlugins := plugins.MergePlugins(agwPlugins...)
	for i, plug := range agwPlugins {
		kube.WaitForCacheSync(fmt.Sprintf("plugin-%d", i), ctx.Done(), plug.HasSynced)
	}
	return mergedPlugins
}

func BuildMockPolicyContext(t test.Failer, inputs []any) plugins.PolicyCtx {
	return plugins.PolicyCtx{
		Krt:         krt.TestingDummyContext{},
		Collections: BuildMockCollection(t, inputs),
	}
}

func BuildMockCollection(t test.Failer, inputs []any) *plugins.AgwCollections {
	mock := krttest.NewMock(t, inputs)
	col := &plugins.AgwCollections{
		Namespaces:           krttest.GetMockCollection[*corev1.Namespace](mock),
		Nodes:                krttest.GetMockCollection[*corev1.Node](mock),
		Pods:                 krttest.GetMockCollection[*corev1.Pod](mock),
		Services:             krttest.GetMockCollection[*corev1.Service](mock),
		Secrets:              krttest.GetMockCollection[*corev1.Secret](mock),
		ConfigMaps:           krttest.GetMockCollection[*corev1.ConfigMap](mock),
		EndpointSlices:       krttest.GetMockCollection[*discovery.EndpointSlice](mock),
		WorkloadEntries:      krttest.GetMockCollection[*networkingclient.WorkloadEntry](mock),
		ServiceEntries:       krttest.GetMockCollection[*networkingclient.ServiceEntry](mock),
		GatewayClasses:       krttest.GetMockCollection[*gwv1.GatewayClass](mock),
		Gateways:             krttest.GetMockCollection[*gwv1.Gateway](mock),
		HTTPRoutes:           krttest.GetMockCollection[*gwv1.HTTPRoute](mock),
		GRPCRoutes:           krttest.GetMockCollection[*gwv1.GRPCRoute](mock),
		TCPRoutes:            krttest.GetMockCollection[*gwv1a2.TCPRoute](mock),
		TLSRoutes:            krttest.GetMockCollection[*gwv1a2.TLSRoute](mock),
		ReferenceGrants:      krttest.GetMockCollection[*gwv1b1.ReferenceGrant](mock),
		BackendTLSPolicies:   krttest.GetMockCollection[*gwv1.BackendTLSPolicy](mock),
		XListenerSets:        krttest.GetMockCollection[*gwxv1a1.XListenerSet](mock),
		InferencePools:       krttest.GetMockCollection[*inf.InferencePool](mock),
		Backends:             krttest.GetMockCollection[*agwv1alpha1.AgentgatewayBackend](mock),
		AgentgatewayPolicies: krttest.GetMockCollection[*agwv1alpha1.AgentgatewayPolicy](mock),
		ControllerName:       wellknown.DefaultAgwControllerName,
		SystemNamespace:      "kgateway-system",
		IstioNamespace:       "istio-system",
		ClusterID:            "Kubernetes",
	}
	col.SetupIndexes()
	return col
}
