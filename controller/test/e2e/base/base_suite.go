//go:build e2e

package base

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	goruntime "runtime"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/Masterminds/semver/v3"
	"github.com/onsi/gomega"
	"istio.io/istio/pkg/config/crd"
	"istio.io/istio/pkg/test"
	istioassert "istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/yml"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/serializer/yaml"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	apitests "github.com/agentgateway/agentgateway/controller/api/tests"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

// GwApiChannel represents the Gateway API release channel
type GwApiChannel string

// Gateway API channel constants
const (
	GwApiChannelStandard     GwApiChannel = "standard"
	GwApiChannelExperimental GwApiChannel = "experimental"
)

const (
	Namespace         = "agentgateway-base"
	WellKnownAppLabel = "app.kubernetes.io/name"
)

// GwApiVersion is its own type to avoid having to import the implementation package in other files.
type GwApiVersion struct {
	semver.Version
}

// GwApiVersionMustParse is a helper function to parse a version string into a GwApiVersion.
func GwApiVersionMustParse(version string) GwApiVersion {
	return GwApiVersion{Version: *semver.MustParse(version)}
}

// Named Gateway API version constants for easy reference
var (
	// HTTPRoutes.spec.rules[].name was added in 1.2.0 experimental (added to standard in 1.4.0)
	GwApiV1_2_0 = GwApiVersionMustParse("1.2.0")
	// BackendTLSPolicy moved to standard/v1 in 1.4.0 and experimental (alpha1v3 version is not supported), HTTPRoutes.spec.rules[].name was added to standard in 1.4.0
	GwApiV1_4_0 = GwApiVersionMustParse("1.4.0")

	GwApiRequireRouteNames = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_2_0,
		GwApiChannelStandard:     &GwApiV1_4_0,
	}

	GwApiRequireBackendTLSPolicy = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_4_0,
		GwApiChannelStandard:     &GwApiV1_4_0,
	}
)

// selfManagedGatewayAnnotation is the annotation used to mark a Gateway as self-managed in e2e tests
const selfManagedGatewayAnnotation = "e2e.agentgateway.dev/self-managed"

type Test struct {
	*testing.T
	Ctx              context.Context
	TestInstallation *e2e.TestInstallation

	// used internally to parse the manifest files
	validator *crd.Validator

	// gwApiVersion stores the detected Gateway API version (detected once and cached)
	gwApiVersion *semver.Version

	// gwApiChannel stores the detected Gateway API channel (detected once and cached)
	gwApiChannel GwApiChannel

	// MinGwApiVersion specifies the minimum Gateway API version required for this entire suite.
	// This is needed on the suite level, because individual tests are skipped after the suite is setup, and the suite setup may apply manifests that are not compatible with the current Gateway API version.
	// Map key is the channel (GwApiChannelStandard or GwApiChannelExperimental), value is the minimum version.
	// If the map is empty/nil, the suite runs on any channel/version.
	// The suite will only run if the Gateway API version is >= the specified minimum version.
	// For minimum requirements, if only experimental constraints exist, the suite is considered experimental-only and will skip on standard channel.
	// Matching logic based on installed channel:
	//   - experimental: If experimental key exists, check version; otherwise run
	//   - standard: If standard key exists, check version; if only experimental exists, skip; otherwise runs on any standard version.
	MinGwApiVersion map[GwApiChannel]*GwApiVersion
}

// SuiteOption is a functional option for configuring Test
type SuiteOption func(*Test)

// WithMinGwApiVersion sets the minimum Gateway API version requirements for the suite
func WithMinGwApiVersion(minVersions map[GwApiChannel]*GwApiVersion) SuiteOption {
	return func(s *Test) {
		s.MinGwApiVersion = minVersions
	}
}

func NewSuite(ctx context.Context, testInst *e2e.TestInstallation, t *testing.T, opts ...SuiteOption) Test {
	suite := Test{
		T:                t,
		Ctx:              ctx,
		TestInstallation: testInst,
	}

	for _, opt := range opts {
		opt(&suite)
	}

	return suite
}

func (s *Test) Run(name string, f func(t Test)) bool {
	s.T.Helper()
	return s.T.Run(name, func(t *testing.T) {
		child := *s
		child.T = t
		f(child)
	})
}

func gatewayAPIMinVersionMatches(requirements map[GwApiChannel]*GwApiVersion, channel GwApiChannel, current GwApiVersion) bool {
	switch channel {
	case GwApiChannelExperimental:
		if requiredVersion, exists := requirements[GwApiChannelExperimental]; exists {
			return current.GreaterThan(&requiredVersion.Version) || current.Equal(&requiredVersion.Version)
		}
		return true

	case GwApiChannelStandard:
		if requiredVersion, exists := requirements[GwApiChannelStandard]; exists {
			return current.GreaterThan(&requiredVersion.Version) || current.Equal(&requiredVersion.Version)
		}
		if _, hasExperimental := requirements[GwApiChannelExperimental]; hasExperimental {
			return false
		}
		return true

	default:
		return false
	}
}

func (s *Test) SetupSuite() {
	// Detect and cache Gateway API version and channel once
	done := traceStep(s, "detected Gateway API version")
	s.detectAndCacheGwApiInfo()
	done()

	// Check suite-level version requirements before proceeding
	if s.SkipSuite() {
		s.Skipf("Test requires Gateway API %s, but current is %s/%s", s.MinGwApiVersion, s.getCurrentGwApiChannel(), s.getCurrentGwApiVersion())
	}

	// set up the helpers once and store them on the suite
	done = traceStep(s, "setup suite helpers")
	s.setupHelpers()
	done()
}

func (s *Test) TearDownSuite() {
}

func (s *Test) applyManifests(manifests ...string) {
	done := func() {}
	if len(manifests) > 0 {
		done = traceStep(s, "applied manifests %v", manifestNames(manifests))
	}
	err := s.TestInstallation.ClusterContext.IstioClient.ApplyYAMLFiles("", manifests...)
	istioassert.NoError(s, err)
	done()

	// parse the expected resources and dynamic resources from the manifests, and wait until the resources are created.
	// we must wait until the resources from the manifest exist on the cluster before calling loadDynamicResources,
	// because in order to determine what dynamic resources are expected, certain resources (e.g. Gateways and
	// GatewayParameters) must already exist on the cluster.
	manifestResources := s.loadManifestResources(manifests...)
	done = traceStep(s, "manifest resources ready")
	s.TestInstallation.AssertionsT(s).EventuallyObjectsExist(s.Ctx, manifestResources...)
	done()
	dynamicResources := s.loadDynamicResources(manifestResources)
	done = traceStep(s, "dynamic resources ready")
	s.TestInstallation.AssertionsT(s).EventuallyObjectsExist(s.Ctx, dynamicResources...)
	done()

	// wait until pods are ready; this assumes that pods use a well-known label
	// app.kubernetes.io/name=<name>
	allResources := slices.Concat(manifestResources, dynamicResources)
	for _, resource := range allResources {
		var ns, name string
		if pod, ok := resource.(*corev1.Pod); ok {
			ns = pod.Namespace
			name = pod.Name
		} else if deployment, ok := resource.(*appsv1.Deployment); ok {
			if deployment.Spec.Replicas != nil && *deployment.Spec.Replicas == 0 {
				continue
			}
			ns = deployment.Namespace
			name = deployment.Name
		} else {
			continue
		}
		done := traceStep(s, "pods ready %s/%s", ns, name)
		s.TestInstallation.AssertionsT(s).EventuallyPodsRunning(s.Ctx, ns, metav1.ListOptions{
			LabelSelector: fmt.Sprintf("%s=%s", WellKnownAppLabel, name),
			// Provide a longer timeout as the pod needs to be pulled and pass HCs
		}, time.Second*60, time.Millisecond*500)
		done()
	}
}

func (s *Test) Apply(manifests ...string) {
	s.Helper()
	if s.SkipSuite() {
		s.Skip("Skipping all tests in suite due to gateway API version requirements")
	}

	s.applyManifests(manifests...)
	s.Cleanup(func() {
		if testutils.ShouldSkipCleanup(s) {
			return
		}
		s.deleteManifests(manifests...)
	})
}

func (s *Test) Delete(manifests ...string) {
	s.Helper()
	s.deleteManifests(manifests...)
}

func Manifest(pathParts ...string) string {
	_, file, _, ok := goruntime.Caller(1)
	if !ok {
		panic("failed to resolve caller for test manifest")
	}
	return filepath.Join(append([]string{filepath.Dir(file), "testdata"}, pathParts...)...)
}

func manifestNames(manifests []string) []string {
	names := make([]string, 0, len(manifests))
	for _, manifest := range manifests {
		names = append(names, filepath.Base(manifest))
	}
	return names
}

func (s *Test) GatewayReady(name, namespace string) {
	s.Helper()
	p := s.TestInstallation.AssertionsT(s)
	p.EventuallyGatewayCondition(s.Ctx, name, namespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	p.EventuallyGatewayCondition(s.Ctx, name, namespace, gwv1.GatewayConditionAccepted, metav1.ConditionTrue)
}

func (s *Test) HTTPRouteAccepted(name, namespace string) {
	s.Helper()
	s.TestInstallation.AssertionsT(s).EventuallyHTTPRouteCondition(s.Ctx, name, namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
}

func (s *Test) Send(target string, expect *testmatchers.HttpResponse, opts ...curl.Option) {
	s.Helper()
	BaseGateway.Send(s, expect, append(targetOptions(s, target), opts...)...)
}

func Expect(status int) *testmatchers.HttpResponse {
	return &testmatchers.HttpResponse{StatusCode: status}
}

func ExpectOK() *testmatchers.HttpResponse {
	return Expect(http.StatusOK)
}

func ExpectForbidden(body gomega.OmegaMatcher) *testmatchers.HttpResponse {
	return &testmatchers.HttpResponse{
		StatusCode: http.StatusForbidden,
		Body:       body,
	}
}

func targetOptions(t test.Failer, target string) []curl.Option {
	t.Helper()
	if target == "" {
		t.Fatal("target must not be empty")
	}

	raw := target
	if !strings.Contains(raw, "://") {
		raw = "http://" + strings.TrimPrefix(raw, "/")
	}
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatalf("invalid request target %q: %v", target, err)
	}
	if u.Host == "" {
		t.Fatalf("invalid request target %q: missing host", target)
	}

	path := strings.TrimPrefix(u.EscapedPath(), "/")
	if u.RawQuery != "" {
		path += "?" + u.RawQuery
	}
	opts := []curl.Option{
		curl.WithHostHeader(u.Host),
		curl.WithPath(path),
	}
	if u.Scheme != "" {
		opts = append(opts, curl.WithScheme(u.Scheme))
	}
	return opts
}

var decUnstructured = yaml.NewDecodingSerializer(unstructured.UnstructuredJSONScheme)

// Deleting namespaces is super super super slow. Avoid deleting them, ever
func stripNamespaceResources(t test.Failer, manifests ...string) string {
	cfgs := []string{}
	for _, manifest := range manifests {
		d, err := os.ReadFile(manifest)
		istioassert.NoError(t, err)
		for _, yml := range yml.SplitString(string(d)) {
			obj := &unstructured.Unstructured{}
			_, gvk, err := decUnstructured.Decode([]byte(yml), nil, obj)
			if runtime.IsMissingKind(err) {
				// Not a k8s object, skip
				continue
			}
			istioassert.NoError(t, err)
			if gvk.Kind != "Namespace" {
				cfgs = append(cfgs, yml)
			}
		}
	}

	return strings.Join(cfgs, "\n---\n")
}

func (s *Test) deleteManifests(manifests ...string) {
	nf := stripNamespaceResources(s, manifests...)
	fp := filepath.Join(s.TestInstallation.GeneratedFiles.TempDir, "delete_manifests.yaml")
	istioassert.NoError(s, os.WriteFile(fp, []byte(nf), 0o644)) //nolint:gosec // G306: Golden test file can be readable

	err := s.TestInstallation.ClusterContext.IstioClient.DeleteYAMLFiles("", fp)
	istioassert.NoError(s, err)
}

func (s *Test) setupHelpers() {
	s.validator = apitests.NewAgentgatewayValidatorSkipMissing(s)
}

func (s *Test) loadManifestResources(manifests ...string) []client.Object {
	var resources []client.Object
	for _, manifest := range manifests {
		objs, err := testutils.LoadFromFiles(manifest, s.TestInstallation.ClusterContext.Client.Scheme(), s.validator)
		istioassert.NoError(s, err)
		resources = append(resources, objs...)
	}
	return resources
}

func (s *Test) loadDynamicResources(manifestResources []client.Object) []client.Object {
	var dynamicResources []client.Object
	for _, obj := range manifestResources {
		if gw, ok := obj.(*gwv1.Gateway); ok {
			selfManaged := IsSelfManagedGateway(gw)

			// if the gateway is not self-managed, then we expect a proxy deployment and service
			// to be created, so add them to the dynamic resource list
			if !selfManaged {
				proxyObjectMeta := metav1.ObjectMeta{
					Name:      gw.GetName(),
					Namespace: gw.GetNamespace(),
				}
				proxyResources := []client.Object{
					&appsv1.Deployment{ObjectMeta: proxyObjectMeta},
					&corev1.Service{ObjectMeta: proxyObjectMeta},
				}
				dynamicResources = append(dynamicResources, proxyResources...)
			}
		}
	}
	return dynamicResources
}

func IsSelfManagedGateway(gw *gwv1.Gateway) bool {
	val, ok := gw.Annotations[selfManagedGatewayAnnotation]
	return ok && strings.EqualFold(val, "true")
}

// detectAndCacheGwApiInfo detects the Gateway API version and channel from installed CRDs
// and caches the results. This is called once during suite setup.
func (s *Test) detectAndCacheGwApiInfo() {
	crd := &apiextensionsv1.CustomResourceDefinition{}
	err := s.TestInstallation.ClusterContext.Client.Get(s.Ctx, client.ObjectKey{Name: "gateways.gateway.networking.k8s.io"}, crd)
	istioassert.NoError(s, err)

	channel, hasChannel := crd.Annotations["gateway.networking.k8s.io/channel"]
	if !hasChannel {
		s.Fatal("Gateway CRD missing 'gateway.networking.k8s.io/channel' annotation")
	}
	s.gwApiChannel = GwApiChannel(channel)

	versionStr, hasVersion := crd.Annotations["gateway.networking.k8s.io/bundle-version"]
	if !hasVersion {
		s.Fatal("Gateway CRD missing 'gateway.networking.k8s.io/bundle-version' annotation")
	}

	version, err := semver.NewVersion(versionStr)
	if err != nil {
		s.Fatalf("failed to parse Gateway API version %q: %v", versionStr, err)
	}
	s.gwApiVersion = version
}

// getCurrentGwApiChannel returns the cached Gateway API channel
func (s *Test) getCurrentGwApiChannel() GwApiChannel {
	return s.gwApiChannel
}

// getCurrentGwApiVersion returns the cached Gateway API version
func (s *Test) getCurrentGwApiVersion() GwApiVersion {
	return GwApiVersion{Version: *s.gwApiVersion}
}

// SkipSuite determines if the entire suite should be skipped based on suite-level minimum version requirements.
func (s *Test) SkipSuite() bool {
	if len(s.MinGwApiVersion) == 0 {
		return false // No requirements = run on any channel/version
	}

	currentVersion := s.getCurrentGwApiVersion()
	currentChannel := s.getCurrentGwApiChannel()

	if currentVersion.Version.String() == "" {
		s.Fatal("cannot determine Gateway API version")
	}

	return !gatewayAPIMinVersionMatches(s.MinGwApiVersion, currentChannel, currentVersion)
}
