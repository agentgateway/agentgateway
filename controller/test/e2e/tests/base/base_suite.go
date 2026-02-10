//go:build e2e

package base

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/Masterminds/semver/v3"
	"github.com/stretchr/testify/suite"
	"istio.io/istio/pkg/maps"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/yml"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	apiserverschema "k8s.io/apiextensions-apiserver/pkg/apiserver/schema"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/runtime/serializer/yaml"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

// GwApiChannel represents the Gateway API release channel
type GwApiChannel string

// Gateway API channel constants
const (
	GwApiChannelStandard     GwApiChannel = "standard"
	GwApiChannelExperimental GwApiChannel = "experimental"
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
	// TLSRoutes and TCPRoutes were added to experimental in 0.3.0. They are not available in standard as of 1.4.0
	GwApiV0_3_0 = GwApiVersionMustParse("0.3.0")
	// SessionPersistence was added in 1.1.0 experimental and is not available in standard as of 1.4.0
	GwApiV1_1_0 = GwApiVersionMustParse("1.1.0")
	// HTTPRoutes.spec.rules[].name was added in 1.2.0 experimental (added to standard in 1.4.0)
	GwApiV1_2_0 = GwApiVersionMustParse("1.2.0")
	// XListenerSets and CORS filters were added in 1.3.0 experimental
	GwApiV1_3_0 = GwApiVersionMustParse("1.3.0")
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

	GwApiRequireListenerSets = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_3_0,
	}

	GwApiRequireCorsFilters = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_3_0,
	}

	GwApiRequireTlsRoutes = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV0_3_0,
	}

	GwApiRequireTcpRoutes = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV0_3_0,
	}

	GwApiRequireSessionPersistence = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_1_0,
	}

	// Gateway.spec.tls.frontend was added in 1.4.0 experimental
	GwApiRequireFrontendTLSConfig = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_4_0,
	}
)

// selfManagedGatewayAnnotation is the annotation used to mark a Gateway as self-managed in e2e tests
const selfManagedGatewayAnnotation = "e2e.kgateway.dev/self-managed"

// TestCase defines the manifests and resources used by a test or test suite.
type TestCase struct {
	// Manifests contains a list of manifest filenames.
	Manifests []string

	// ManifestsWithTransform maps a manifest filename to a function that transforms its contents before applying it
	ManifestsWithTransform map[string]func(string) string

	// manifestResources contains the resources automatically loaded from the manifest files for
	// this test case.
	manifestResources []client.Object

	// dynamicResources contains the expected dynamically provisioned resources for any Gateways
	// contained in this test case's manifests.
	dynamicResources []client.Object

	// MinGwApiVersion specifies the minimum Gateway API version required per channel.
	// Map key is the channel (GwApiChannelStandard or GwApiChannelExperimental), value is the minimum version.
	// If the map is empty/nil, the test runs on any channel/version.
	// The test will only run if the Gateway API version is >= the specified minimum version.
	// For minimum requirements, if only experimental constraints exist, the test is considered experimental-only and will skip on standard channel.
	// Matching logic based on installed channel:
	//   - experimental: If experimental key exists, check version; otherwise run
	//   - standard: If standard key exists, check version; if only experimental exists, skip; otherwise runs on any standard version.
	MinGwApiVersion map[GwApiChannel]*GwApiVersion

	// MaxGwApiVersion specifies the maximum Gateway API version required per channel.
	// Map key is the channel (GwApiChannelStandard or GwApiChannelExperimental), value is the maximum version.
	// If the map is empty/nil, the test runs on any channel/version.
	// The test will only run if the Gateway API version is < the specified maximum version.
	// Maximum constraints are channel-specific - experimental constraints don't affect standard channel execution.
	// If the maximum version is less than the minimum version, the test will be skipped.
	MaxGwApiVersion map[GwApiChannel]*GwApiVersion
}

type BaseTestingSuite struct {
	suite.Suite
	Ctx              context.Context
	TestInstallation *e2e.TestInstallation
	Setup            TestCase
	TestCases        map[string]*TestCase

	// (Optional) Path of directory (relative to git root) containing the CRDs that will be used to read
	// the objects from the manifests. If empty then defaults to "install/helm/kgateway-crds/templates"
	CrdPath string

	// used internally to parse the manifest files
	gvkToStructuralSchema map[schema.GroupVersionKind]*apiserverschema.Structural

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

	// setupByVersion allows defining different setup configurations for different GW API versions and channels.
	// The outer map key is the channel (standard or experimental).
	// The inner map key is the minimum version, and the value is the TestCase to use.
	// The system will select the setup with the highest matching version for the current channel.
	// If no setups match, falls back to the Setup field.
	// Example:
	//   setupByVersion: map[GwApiChannel]map[KGatewayVersion]*TestCase{
	//     GwApiChannelExperimental: {
	//       GwApiV1_3_0: &setupExperimentalV1_4,
	//     },
	//     GwApiChannelStandard: {
	//       GwApiV1_3_0: &setupStandardV1_4,
	//     },
	//   }
	setupByVersion map[GwApiChannel]map[GwApiVersion]*TestCase

	// selectedSetup tracks which setup was actually used, so we can clean it up in TearDownSuite
	selectedSetup *TestCase
}

// SuiteOption is a functional option for configuring BaseTestingSuite
type SuiteOption func(*BaseTestingSuite)

// WithMinGwApiVersion sets the minimum Gateway API version requirements for the suite
func WithMinGwApiVersion(minVersions map[GwApiChannel]*GwApiVersion) SuiteOption {
	return func(s *BaseTestingSuite) {
		s.MinGwApiVersion = minVersions
	}
}

// WithSetupByVersion sets version-specific setup configurations for the suite
func WithSetupByVersion(setupByVersion map[GwApiChannel]map[GwApiVersion]*TestCase) SuiteOption {
	return func(s *BaseTestingSuite) {
		s.setupByVersion = setupByVersion
	}
}

func WithCrdPath(crdPath string) SuiteOption {
	return func(s *BaseTestingSuite) {
		s.CrdPath = crdPath
	}
}

// NewBaseTestingSuite returns a BaseTestingSuite that performs all the pre-requisites of upgrading helm installations,
// applying manifests and verifying resources exist before a suite and tests and the corresponding post-run cleanup.
// The pre-requisites for the suite are defined in the setup parameter and for each test in the individual testCase.
func NewBaseTestingSuite(ctx context.Context, testInst *e2e.TestInstallation, setupTestCase TestCase, testCases map[string]*TestCase, opts ...SuiteOption) *BaseTestingSuite {
	suite := &BaseTestingSuite{
		Ctx:              ctx,
		TestInstallation: testInst,
		Setup:            setupTestCase,
		TestCases:        testCases,
	}

	for _, opt := range opts {
		opt(suite)
	}

	return suite
}

// versionChecker is a function type for checking version constraints
type versionChecker func(current, required GwApiVersion) bool

// getChannelRequirements returns the version checker logic for a specific channel
func getChannelRequirements(requirements map[GwApiChannel]*GwApiVersion, channel GwApiChannel, checker versionChecker, isMinRequirement bool) func(GwApiVersion) bool {
	switch channel {
	case GwApiChannelExperimental:
		if requiredVersion, exists := requirements[GwApiChannelExperimental]; exists {
			return func(currentVersion GwApiVersion) bool {
				return checker(currentVersion, *requiredVersion)
			}
		}
		return func(GwApiVersion) bool { return true } // No experimental requirements = matches any experimental

	case GwApiChannelStandard:
		if requiredVersion, exists := requirements[GwApiChannelStandard]; exists {
			return func(currentVersion GwApiVersion) bool {
				return checker(currentVersion, *requiredVersion)
			}
		}
		// If experimental defined but not standard - don't match (test uses experimental-only features)
		// This logic only applies to minimum requirements, not maximum requirements
		if isMinRequirement {
			if _, hasExperimental := requirements[GwApiChannelExperimental]; hasExperimental {
				return func(GwApiVersion) bool { return false }
			}
		}
		return func(GwApiVersion) bool { return true } // No requirements = matches any standard

	default:
		return func(GwApiVersion) bool { return false } // Unknown channel
	}
}

// checkCompatibleWithApiVersion checks if the requirements for a test are satisfied by the current channel/version.
func (s *BaseTestingSuite) checkCompatibleWithApiVersion(minRequirements, maxRequirements map[GwApiChannel]*GwApiVersion, currentChannel GwApiChannel, currentVersion GwApiVersion) bool {
	minChecker := func(current, required GwApiVersion) bool {
		return current.GreaterThan(&required.Version) || current.Equal(&required.Version) // >=
	}
	maxChecker := func(current, required GwApiVersion) bool {
		return current.LessThan(&required.Version) // <
	}

	minConstraint := getChannelRequirements(minRequirements, currentChannel, minChecker, true)
	maxConstraint := getChannelRequirements(maxRequirements, currentChannel, maxChecker, false)

	return minConstraint(currentVersion) && maxConstraint(currentVersion)
}

// selectSetup chooses the appropriate setup TestCase based on the current Gateway API version and channel.
// If SetupByVersion is defined, it selects the setup with the highest matching version requirement.
// Otherwise, it returns the default Setup.
func (s *BaseTestingSuite) selectSetup() *TestCase {
	// If versioned setups are not defined, use the default Setup
	if len(s.setupByVersion) == 0 {
		return &s.Setup
	}

	currentVersion := s.getCurrentGwApiVersion()
	currentChannel := s.getCurrentGwApiChannel()

	if currentVersion.Version.String() == "" {
		// Can't determine version, something is wrong
		s.Require().FailNow("cannot determine Gateway API version")
	}

	// Get the version map for the current channel
	versionMap, hasChannel := s.setupByVersion[currentChannel]
	if !hasChannel || len(versionMap) == 0 {
		// No setups defined for this channel, fall back to default
		return &s.Setup
	}

	// Find the highest version that's <= current version
	// Get all version keys and sort in descending order (highest first)
	var versions []GwApiVersion
	for version := range versionMap {
		versions = append(versions, version)
	}
	slices.SortFunc(versions, func(a, b GwApiVersion) int {
		return a.Compare(&b.Version) * -1 // sort in descending order
	})

	// Find the first (highest) version that satisfies the requirement
	for _, minVersion := range versions {
		if currentVersion.GreaterThan(&minVersion.Version) || currentVersion.Equal(&minVersion.Version) {
			return versionMap[minVersion]
		}
	}

	// Fallback to default Setup if no match
	return &s.Setup
}

func (s *BaseTestingSuite) SetupSuite() {
	// Detect and cache Gateway API version and channel once
	s.detectAndCacheGwApiInfo()

	// Check suite-level version requirements before proceeding
	if s.SkipSuite() {
		// There isn't a way to skip the whole suite, but still need to check here to avoid the setup of potentially incompatible resources.
		s.T().Logf("Suite requires Gateway API %s, but current is %s/%s", s.MinGwApiVersion, s.getCurrentGwApiChannel(), s.getCurrentGwApiVersion())
		return
	}

	// set up the helpers once and store them on the suite
	s.setupHelpers()

	// Select the appropriate setup based on Gateway API version
	s.selectedSetup = s.selectSetup()
	s.ApplyManifests(s.selectedSetup)
}

func (s *BaseTestingSuite) TearDownSuite() {
	if testutils.ShouldSkipCleanup(s.T()) || s.SkipSuite() {
		return
	}

	// Use the selected setup if available, otherwise fall back to default Setup
	setupToDelete := s.selectedSetup
	s.DeleteManifests(setupToDelete)
}

func (s *BaseTestingSuite) BeforeTest(suiteName, testName string) {
	// Check first if the suite should be skipped due to version requirements to cover cases when the testcase is not defined.
	if s.SkipSuite() {
		s.T().Skip("Skipping all tests in suite due to gateway API version requirements")
	}

	// apply test-specific manifests
	testCase, ok := s.TestCases[testName]
	if !ok {
		return
	}

	// Check version requirements before applying manifests
	if shouldSkip := s.skipTest(testCase); shouldSkip {
		s.T().Skipf("Test requires Gateway API %s, but current is %s/%s",
			testCase.MinGwApiVersion, s.getCurrentGwApiChannel(), s.getCurrentGwApiVersion())
		return
	}

	s.ApplyManifests(testCase)
}

func (s *BaseTestingSuite) AfterTest(suiteName, testName string) {
	if s.T().Failed() && !testutils.ShouldSkipBugReport() {
		s.TestInstallation.PerTestPreFailHandler(s.Ctx, s.T(), testName)
	}

	// Delete test-specific manifests
	testCase, ok := s.TestCases[testName]
	if !ok {
		return
	}

	// Check if the test was skipped due to version requirements
	// If so, don't try to delete resources that were never applied
	if s.skipTest(testCase) || s.SkipSuite() {
		return
	}

	if testutils.ShouldSkipCleanup(s.T()) {
		return
	}
	s.DeleteManifests(testCase)
}

func (s *BaseTestingSuite) GetKubectlOutput(command ...string) string {
	out, _, err := s.TestInstallation.Actions.Kubectl().Execute(s.Ctx, command...)
	s.TestInstallation.AssertionsT(s.T()).Require.NoError(err)

	return out
}

// ApplyManifests applies the manifests and waits until the resources are created and ready.
func (s *BaseTestingSuite) ApplyManifests(testCase *TestCase) {
	// apply the manifests
	err := s.TestInstallation.ClusterContext.IstioClient.ApplyYAMLFiles("", testCase.Manifests...)
	s.Require().NoError(err, "manifests %v", testCase.Manifests)

	for manifest, transform := range testCase.ManifestsWithTransform {
		cur, err := os.ReadFile(manifest)
		s.Require().NoError(err)
		transformed := transform(string(cur))
		err = s.TestInstallation.ClusterContext.IstioClient.ApplyYAMLContents("", transformed)
		s.Require().NoError(err)
	}

	// parse the expected resources and dynamic resources from the manifests, and wait until the resources are created.
	// we must wait until the resources from the manifest exist on the cluster before calling loadDynamicResources,
	// because in order to determine what dynamic resources are expected, certain resources (e.g. Gateways and
	// GatewayParameters) must already exist on the cluster.
	s.loadManifestResources(testCase)
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.Ctx, testCase.manifestResources...)
	s.loadDynamicResources(testCase)
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.Ctx, testCase.dynamicResources...)

	// wait until pods are ready; this assumes that pods use a well-known label
	// app.kubernetes.io/name=<name>
	allResources := slices.Concat(testCase.manifestResources, testCase.dynamicResources)
	for _, resource := range allResources {
		var ns, name string
		if pod, ok := resource.(*corev1.Pod); ok {
			ns = pod.Namespace
			name = pod.Name
		} else if deployment, ok := resource.(*appsv1.Deployment); ok {
			ns = deployment.Namespace
			name = deployment.Name
		} else {
			continue
		}
		s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(s.Ctx, ns, metav1.ListOptions{
			LabelSelector: fmt.Sprintf("%s=%s", defaults.WellKnownAppLabel, name),
			// Provide a longer timeout as the pod needs to be pulled and pass HCs
		}, time.Second*60, time.Millisecond*500)
	}
}

var decUnstructured = yaml.NewDecodingSerializer(unstructured.UnstructuredJSONScheme)

// Deleting namespaces is super super super slow. Avoid deleting them, ever
func stripNamespaceResources(t *testing.T, manifests ...string) string {
	cfgs := []string{}
	for _, manifest := range manifests {
		d, err := os.ReadFile(manifest)
		assert.NoError(t, err)
		for _, yml := range yml.SplitString(string(d)) {
			obj := &unstructured.Unstructured{}
			_, gvk, err := decUnstructured.Decode([]byte(yml), nil, obj)
			if runtime.IsMissingKind(err) {
				// Not a k8s object, skip
				continue
			}
			assert.NoError(t, err)
			if gvk.Kind != "Namespace" {
				cfgs = append(cfgs, yml)
			}
		}
	}

	return strings.Join(cfgs, "\n---\n")
}

// DeleteManifests deletes the manifests and waits until the resources are deleted.
func (s *BaseTestingSuite) DeleteManifests(testCase *TestCase) {
	nf := stripNamespaceResources(s.T(), testCase.Manifests...)
	fp := filepath.Join(s.TestInstallation.GeneratedFiles.TempDir, "delete_manifests.yaml")
	s.Require().NoError(os.WriteFile(fp, []byte(nf), 0o644)) //nolint:gosec // G306: Golden test file can be readable

	err := s.TestInstallation.ClusterContext.IstioClient.DeleteYAMLFiles("", fp)
	s.Require().NoError(err)

	// we don't need to transform the manifest here, as we are just deleting by filename
	err = s.TestInstallation.ClusterContext.IstioClient.DeleteYAMLFiles("", maps.Keys(testCase.ManifestsWithTransform)...)
	s.Require().NoError(err)
}

func (s *BaseTestingSuite) setupHelpers() {
	if s.CrdPath == "" {
		s.CrdPath = testutils.AgwCRDPath
	}
	var err error
	s.gvkToStructuralSchema, err = testutils.GetStructuralSchemasForBothCharts()
	s.Require().NoError(err)
}

// loadManifestResources populates the `manifestResources` for the given test case, by parsing each
// manifest file into a list of resources
func (s *BaseTestingSuite) loadManifestResources(testCase *TestCase) {
	if len(testCase.manifestResources) > 0 {
		// resources have already been loaded
		return
	}

	var resources []client.Object
	for _, manifest := range testCase.Manifests {
		objs, err := testutils.LoadFromFiles(manifest, s.TestInstallation.ClusterContext.Client.Scheme(), s.gvkToStructuralSchema)
		s.Require().NoError(err)
		resources = append(resources, objs...)
	}
	for manifest, transform := range testCase.ManifestsWithTransform {
		// we don't need to transform the resource since the transformation applies to the spec and not object metadata,
		// which ensures that parsed Go objects in manifestResources can be used normally
		objs, err := testutils.LoadFromFileWithTransform(manifest, s.TestInstallation.ClusterContext.Client.Scheme(), s.gvkToStructuralSchema, transform)
		s.Require().NoError(err)
		resources = append(resources, objs...)
	}
	testCase.manifestResources = resources
}

// loadDynamicResources populates the `dynamicResources` for the given test case. For each Gateway
// in the test case, if it is not self-managed, then the expected dynamically provisioned resources
// are added to dynamicResources.
//
// This should only be called *after* loadManifestResources has been called and we have waited
// for all the manifest objects to be created. This is because the "is self-managed" check requires
// any dependent Gateways and GatewayParameters to exist on the cluster already.
func (s *BaseTestingSuite) loadDynamicResources(testCase *TestCase) {
	if len(testCase.dynamicResources) > 0 {
		// resources have already been loaded
		return
	}

	var dynamicResources []client.Object
	for _, obj := range testCase.manifestResources {
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
	testCase.dynamicResources = dynamicResources
}

func IsSelfManagedGateway(gw *gwv1.Gateway) bool {
	val, ok := gw.Annotations[selfManagedGatewayAnnotation]
	return ok && strings.EqualFold(val, "true")
}

// detectAndCacheGwApiInfo detects the Gateway API version and channel from installed CRDs
// and caches the results. This is called once during suite setup.
func (s *BaseTestingSuite) detectAndCacheGwApiInfo() {
	crd := &apiextensionsv1.CustomResourceDefinition{}
	err := s.TestInstallation.ClusterContext.Client.Get(s.Ctx, client.ObjectKey{Name: "gateways.gateway.networking.k8s.io"}, crd)
	s.Require().NoError(err, "failed to get Gateway CRD to detect Gateway API version/channel")

	channel, hasChannel := crd.Annotations["gateway.networking.k8s.io/channel"]
	s.Require().True(hasChannel, "Gateway CRD missing 'gateway.networking.k8s.io/channel' annotation")
	s.gwApiChannel = GwApiChannel(channel)

	versionStr, hasVersion := crd.Annotations["gateway.networking.k8s.io/bundle-version"]
	s.Require().True(hasVersion, "Gateway CRD missing 'gateway.networking.k8s.io/bundle-version' annotation")

	version, err := semver.NewVersion(versionStr)
	s.Require().NoError(err, "failed to parse Gateway API version '%s'", versionStr)
	s.gwApiVersion = version
}

// getCurrentGwApiChannel returns the cached Gateway API channel
func (s *BaseTestingSuite) getCurrentGwApiChannel() GwApiChannel {
	return s.gwApiChannel
}

// getCurrentGwApiVersion returns the cached Gateway API version
func (s *BaseTestingSuite) getCurrentGwApiVersion() GwApiVersion {
	return GwApiVersion{Version: *s.gwApiVersion}
}

// skipTest determines if a test should be skipped based on channel/version requirements.
// This is the inverse of requirementsMatch - we skip if requirements are NOT met.
func (s *BaseTestingSuite) skipTest(testCase *TestCase) bool {
	if len(testCase.MinGwApiVersion) == 0 && len(testCase.MaxGwApiVersion) == 0 {
		return false // No requirements = run on any channel/version
	}

	currentVersion := s.getCurrentGwApiVersion()
	currentChannel := s.getCurrentGwApiChannel()

	if currentVersion.Version.String() == "" {
		s.Require().FailNow("cannot determine Gateway API version")
	}

	// Use checkCompatibleWithApiVersion and invert the result
	return !s.checkCompatibleWithApiVersion(testCase.MinGwApiVersion, testCase.MaxGwApiVersion, currentChannel, currentVersion)
}

// SkipSuite determines if the entire suite should be skipped based on suite-level minimum version requirements.
func (s *BaseTestingSuite) SkipSuite() bool {
	if len(s.MinGwApiVersion) == 0 {
		return false // No requirements = run on any channel/version
	}

	currentVersion := s.getCurrentGwApiVersion()
	currentChannel := s.getCurrentGwApiChannel()

	if currentVersion.Version.String() == "" {
		s.Require().FailNow("cannot determine Gateway API version")
	}

	// Use checkCompatibleWithApiVersion with empty max requirements (only check min)
	return !s.checkCompatibleWithApiVersion(s.MinGwApiVersion, nil, currentChannel, currentVersion)
}
