package deployer

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"k8s.io/apimachinery/pkg/runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"
	"sigs.k8s.io/yaml"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	pkgdeployer "github.com/kgateway-dev/kgateway/v2/pkg/deployer"
	internaldeployer "github.com/kgateway-dev/kgateway/v2/pkg/kgateway/deployer"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/collections"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/envutils"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

type HelmTestCase struct {
	Name   string
	Inputs *pkgdeployer.Inputs
	// InputFile is just the name of the manifest omitting the file extension suffix
	InputFile string
	// Validate is an optional function to run additional validation on the output YAML
	Validate func(t *testing.T, outputYaml string)
	// HelmValuesGeneratorOverride is an optional function to modify deployer inputs before rendering.
	// This is useful for tests that need special configuration like TLS.
	HelmValuesGeneratorOverride func(inputs *pkgdeployer.Inputs) pkgdeployer.HelmValuesGenerator
}

type DeployerTester struct {
	AgwControllerName string
	AgwClassName      string
}

// EmptySecurityContextValidator returns a validation function that allows
// securityContext: {} but ensures no actual security values are configured.
// Use this for $patch: delete, which sets securityContext to empty struct.
// OpenShift treats an empty securityContext the same as a nonexistent one --
// the SCC (by default, `restricted-v2`) will fill in the securityContext.
func EmptySecurityContextValidator() func(t *testing.T, outputYaml string) {
	return func(t *testing.T, outputYaml string) {
		t.Helper()
		// These are actual security values that should NOT be present when using $patch: delete
		forbiddenValues := []string{
			"runAsUser:",
			"runAsGroup:",
			"runAsNonRoot:",
			"allowPrivilegeEscalation:",
			"capabilities:",
			"readOnlyRootFilesystem:",
			"privileged:",
			"fsGroup:",
			"supplementalGroups:",
		}
		for _, val := range forbiddenValues {
			assert.NotContains(t, outputYaml, val,
				"output YAML should not contain security values when $patch: delete is used, found: %s", val)
		}
	}
}

// VerifyAllYAMLFilesReferenced ensures every YAML file in testDataDir has a corresponding test case.
// The exclude parameter allows skipping files that are tested elsewhere (e.g., TLS tests).
func VerifyAllYAMLFilesReferenced(t *testing.T, testDataDir string, testCases []HelmTestCase, exclude ...string) {
	t.Helper()
	if envutils.IsEnvTruthy("REFRESH_GOLDEN") {
		t.Log("Skipping reference validation because REFRESH_GOLDEN is set")
		return
	}
	yamlFiles, err := filepath.Glob(filepath.Join(testDataDir, "*.yaml"))
	require.NoError(t, err, "failed to list YAML files in %s", testDataDir)

	referencedFiles := make(map[string]bool)
	for _, tc := range testCases {
		referencedFiles[tc.InputFile] = true
	}
	for _, excl := range exclude {
		referencedFiles[excl] = true
	}

	var unreferenced []string
	for _, yamlFile := range yamlFiles {
		baseName := filepath.Base(yamlFile)
		// Skip golden files
		if strings.HasSuffix(baseName, "-out.yaml") {
			continue
		}
		inputName := strings.TrimSuffix(baseName, ".yaml")
		if !referencedFiles[inputName] {
			unreferenced = append(unreferenced, baseName)
		}
	}

	require.Empty(t, unreferenced, "Found YAML files in %s without corresponding test cases: %v", testDataDir, unreferenced)
}

// ExtractCommonObjs will return a collection containing only objects necessary for collections.CommonCollections,
// so we don't add unknown objects to avoid logging from krttest package re: objects not consumed
func ExtractCommonObjs(t *testing.T, objs []client.Object) ([]client.Object, *gwv1.Gateway) {
	var commonObjs []client.Object
	var gtw *gwv1.Gateway
	for i := range objs {
		switch obj := objs[i].(type) {
		case *gwv1.Gateway:
			if gtw != nil {
				t.Log("found extraneous Gateway in input manifests", client.ObjectKeyFromObject(obj).String())
				t.Fail()
			}
			gtw = obj
			commonObjs = append(commonObjs, gtw)
		case *gwv1.GatewayClass:
			commonObjs = append(commonObjs, obj)
		case *gwxv1a1.XListenerSet:
			commonObjs = append(commonObjs, obj)
		}
	}
	return commonObjs, gtw
}

func (dt DeployerTester) GetObjects(
	t *testing.T,
	tt HelmTestCase,
	scheme *runtime.Scheme,
	dir string,
	crdDir string,
) []client.Object {
	filePath := filepath.Join(dir, "testdata/", tt.InputFile)
	inputFile := filePath + ".yaml"

	gvkToStructuralSchema, err := testutils.GetStructuralSchemas(crdDir)
	require.NoError(t, err, "error getting structural schemas")

	objs, err := testutils.LoadFromFiles(inputFile, scheme, gvkToStructuralSchema)
	require.NoError(t, err, "error loading files from input file")

	return objs
}

func (dt DeployerTester) RunHelmChartTest(
	t *testing.T,
	tt HelmTestCase,
	scheme *runtime.Scheme,
	dir string,
	crdDir string,
	fakeClient apiclient.Client,
) {
	filePath := filepath.Join(dir, "testdata/", tt.InputFile)
	outputFile := filePath + "-out.yaml"

	objs := dt.GetObjects(t, tt, scheme, dir, crdDir)

	_, gtw := ExtractCommonObjs(t, objs)
	if gtw == nil {
		t.Log("No Gateway found in test files, failing...")
		t.FailNow()
	}
	commonCols := NewCommonCols(t)
	inputs := DefaultDeployerInputs(dt, commonCols)
	if tt.Inputs != nil {
		inputs = tt.Inputs
	}

	gwParams := internaldeployer.NewGatewayParameters(
		fakeClient,
		inputs,
	)
	if tt.HelmValuesGeneratorOverride != nil {
		gwParams.WithHelmValuesGeneratorOverride(tt.HelmValuesGeneratorOverride(inputs))
	}
	deployer, err := internaldeployer.NewGatewayDeployer(
		dt.AgwControllerName,
		dt.AgwClassName,
		scheme,
		fakeClient,
		gwParams,
	)
	assert.NoError(t, err, "error creating gateway deployer")

	ctx := t.Context()
	fakeClient.RunAndWait(ctx.Done())

	// Get post-processed objects (what actually gets deployed)
	deployObjs, err := deployer.GetObjsToDeploy(ctx, gtw)
	assert.NoError(t, err, "error getting objects to deploy")

	got, err := objectsToYAML(deployObjs)
	assert.NoError(t, err, "error converting objects to YAML")

	got = sanitizeOutput(got)

	if envutils.IsEnvTruthy("REFRESH_GOLDEN") {
		t.Log("REFRESH_GOLDEN is set, writing output file", outputFile)
		err = os.WriteFile(outputFile, got, 0o644) //nolint:gosec // G306: Golden test file can be readable
		assert.NoError(t, err, "error writing output file")
	}
	data, err := os.ReadFile(outputFile)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			t.Log("outputFile does not exist, writing output file", outputFile)
			err = os.WriteFile(outputFile, got, 0o644) //nolint:gosec // G306: Golden test file can be readable
			assert.NoError(t, err, "error writing output file")
			t.FailNow()
		} else {
			t.Log("could not read outputFile", outputFile, err)
			t.FailNow()
		}
	}

	// Validate the output YAML
	validateYAML(t, outputFile, data)

	diff := cmp.Diff(data, got)
	outputStr := "%s\nthe golden file, which can be refreshed via `REFRESH_GOLDEN=true go test ./test/deployer`, is\n%s"
	assert.Empty(t, diff, outputStr, diff, outputFile)

	// Run additional validation if provided
	if tt.Validate != nil {
		tt.Validate(t, string(data))
	}
}

// Remove things that change often but are not relevant to the tests
func sanitizeOutput(got []byte) []byte {
	old := fmt.Sprintf("%s/%s:%v", pkgdeployer.AgentgatewayRegistry, pkgdeployer.AgentgatewayImage, pkgdeployer.AgentgatewayDefaultTag)
	now := fmt.Sprintf("%s/%s:99.99.99", pkgdeployer.AgentgatewayRegistry, pkgdeployer.AgentgatewayImage)

	return bytes.Replace(got, []byte(old), []byte(now), -1)
}

// objectsToYAML converts a slice of client.Object to YAML bytes, separated by "---"
func objectsToYAML(objs []client.Object) ([]byte, error) {
	var result []byte
	for i, obj := range objs {
		objYAML, err := yaml.Marshal(obj)
		if err != nil {
			return nil, err
		}
		if i > 0 {
			result = append(result, []byte("---\n")...)
		}
		result = append(result, objYAML...)
	}
	return result, nil
}

func DefaultDeployerInputs(dt DeployerTester, commonCols *collections.CommonCollections) *pkgdeployer.Inputs {
	return &pkgdeployer.Inputs{
		Dev:               false,
		CommonCollections: commonCols,
		ControlPlane: pkgdeployer.ControlPlaneInfo{
			XdsHost:    "xds.cluster.local",
			AgwXdsPort: 9978,
		},
		AgentgatewayClassName:      dt.AgwClassName,
		AgentgatewayControllerName: dt.AgwControllerName,
	}
}

// validateYAML checks that the YAML file is valid:
//
// 1. No lines should end with whitespace. This sometimes means you forgot
// something important, such as handling edge cases when certain values are
// absent, e.g. `foo: {{ bar }}` leads to `foo: ` if bar is the empty string,
// and should probably be handled by omitting `foo:` altogether. If it's not a
// real problem, it's lint that can be quickly fixed.
//
// 2. No blank lines appear, which helps ensure good hygiene around usage of
// helm `indent` and `nindent` template functions.
//
// 3. Each YAML document should round-trip: unmarshaling and then marshaling
// should produce the same YAML content.
func validateYAML(t *testing.T, filename string, data []byte) {
	t.Helper()

	// Check for (1,2)
	lines := strings.Split(string(data), "\n")
	for i, line := range lines {
		if strings.HasSuffix(line, " ") || strings.HasSuffix(line, "\t") {
			t.Errorf("helm chart produced yaml indicative of a buggy helm chart not handling edge cases: line %d in %s ends with whitespace: %q", i+1, filename, line)
		}
		if line == "" && i+1 != len(lines) {
			t.Errorf("helm chart produced yaml with blank lines; please remove these to help ensure good hygiene around usage of helm template functions indent/nindent: line %d in %s is blank", i+1, filename)
		}
	}

	// (3) Split into YAML documents and verify each one round-trips
	documents := strings.Split(string(data), "\n---\n")
	for i, doc := range documents {
		doc = strings.TrimSpace(doc)
		if doc == "" || doc == "---" {
			continue
		}
		var obj any
		if err := yaml.Unmarshal([]byte(doc), &obj); err != nil {
			t.Errorf("helm chart produced invalid yaml: failed to unmarshal document %d in %s: %v", i+1, filename, err)
			continue
		}

		// Verify round-trip: marshal back to YAML and compare
		marshaled, err := yaml.Marshal(obj)
		if err != nil {
			t.Errorf("helm chart produced yaml that cannot be marshaled back: failed to marshal document %d in %s: %v", i+1, filename, err)
			continue
		}

		// Unmarshal round-trip and compare objects (handles key ordering)
		var objRoundTrip any
		if err := yaml.Unmarshal(marshaled, &objRoundTrip); err != nil {
			t.Errorf("helm chart produced yaml that fails to unmarshal after round-trip: document %d in %s: %v", i+1, filename, err)
			continue
		}

		// First check: objects should be semantically equal (order-independent)
		if diff := cmp.Diff(obj, objRoundTrip); diff != "" {
			t.Errorf("helm chart produced yaml that does not round-trip semantically: document %d in %s\nDiff (- original, + after round-trip):\n%s", i+1, filename, diff)
			continue
		}

		// Second check: string comparison to catch implicit null becoming
		// explicit, which could be a mistaken use of '{{- with ... }}'
		// Normalize whitespace but preserve the actual YAML syntax differences
		originalNorm := strings.TrimSpace(doc)
		marshaledNorm := strings.TrimSpace(string(marshaled))

		// Check if marshaled version has explicit 'null' that wasn't in original
		if !strings.Contains(originalNorm, ": null") && strings.Contains(marshaledNorm, ": null") {
			diff := cmp.Diff(originalNorm, marshaledNorm)
			t.Errorf("helm chart produced yaml with implicit null that becomes explicit: document %d in %s\nDiff (- original, + after round-trip):\n%s", i+1, filename, diff)
		}
	}
}
