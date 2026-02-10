package deployer

// This test suite validates helm chart rendering and post-processing with
// overlays (strategic-merge-patch) for managed Gateway deployments.
//
// # Fake Client and Server-Side Apply Semantics
//
// The fake client used in these tests (no need for envtest, which is slower
// and still not as thorough as an e2e test) preserves null values in CRD
// fields marked with x-kubernetes-preserve-unknown-fields, mimicking the
// behavior of `kubectl apply --server-side`. This differs from regular
// client-side `kubectl apply`, which strips null values before sending them to
// the API server.
//
// This means tests here accurately reflect what happens when users apply
// AgentgatewayParameters with `kubectl apply --server-side`, helm 4 in default
// `--server-side` mode, Argo CD with ServerSideApply set to true, etc. If a
// user uses regular `kubectl apply` with null values in overlay fields, the
// nulls will be stripped and the strategic merge patch won't see them. That's
// why our API docs say to prefer using `$patch: delete` instead of null values
// when removing fields. See the API documentation for
// KubernetesResourceOverlay.Spec for details.

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient/fake"
	pkgdeployer "github.com/kgateway-dev/kgateway/v2/pkg/deployer"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/schemes"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/version"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

func mockVersion(t *testing.T) {
	// Save the original version and restore it after the test
	// This ensures the test uses a fixed version (1.0.0-ci1) regardless of
	// what VERSION was set when compiling the test binary
	originalVersion := version.Version
	version.Version = "1.0.0-ci1"
	t.Cleanup(func() {
		version.Version = originalVersion
	})
}

func TestRenderHelmChart(t *testing.T) {
	mockVersion(t)

	// Create temporary CA certificate file for TLS tests
	caCertContent := `-----BEGIN CERTIFICATE-----
MIICljCCAX4CCQCKSGhvPtMNGzANBgkqhkiG9w0BAQsFADANMQswCQYDVQQGEwJV
UzAeFw0yNDA3MDEwMDAwMDBaFw0yNTA3MDEwMDAwMDBaMA0xCzAJBgNVBAYTAlVT
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA1234567890ABCDEFGHIj
klmnopqrstuvwxyz1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890ab
cdefghijklmnopqrstuvwxyz1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ123456
7890abcdefghijklmnopqrstuvwxyz1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ
1234567890abcdefghijklmnopqrstuvwxyz1234567890ABCDEFGHIJKLMNOPQRSTU
VWXYZ1234567890abcdefghijklmnopqrstuvwxyz1234567890ABCDEFGHIJKLMNO
PQRSTUVWXYZ1234567890abcdefghijklmnopqrstuvwxyz1234567890ABCDEFGHI
JKLMNOPQRSTUVWXYZ1234567890abcdefghijklmnopqrstuvwxyz1234567890ABC
DEFGHIJKLMNOPQRSTUVWXYZ1234567890abcdefghijklmnopqrstuvwxyz123456
wIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQBtestcertdata
-----END CERTIFICATE-----`
	tmpDir := t.TempDir()
	caCertPath := tmpDir + "/ca.crt"
	err := os.WriteFile(caCertPath, []byte(caCertContent), 0o600)
	require.NoError(t, err)

	// TLS override function for tests that need TLS enabled
	tlsOverride := func(caCertPath string) func(inputs *pkgdeployer.Inputs) pkgdeployer.HelmValuesGenerator {
		return func(inputs *pkgdeployer.Inputs) pkgdeployer.HelmValuesGenerator {
			inputs.ControlPlane.XdsTLS = true
			inputs.ControlPlane.XdsTlsCaPath = caCertPath
			return nil
		}
	}

	tests := []HelmTestCase{
		{
			Name:      "agentgateway",
			InputFile: "agentgateway",
		},
		{
			// Uses $patch: delete for pod-level and null for container-level securityContext
			Name:      "agentgateway omit securityContext via $patch:delete and null AGWP via GWC",
			InputFile: "agentgateway-omitdefaultsecuritycontext",
			Validate:  EmptySecurityContextValidator(),
		},
		{
			// Uses null for pod-level and $patch: delete for container-level securityContext
			Name:      "agentgateway omit securityContext via null and $patch:delete AGWP via GW",
			InputFile: "agentgateway-omitdefaultsecuritycontext-ref-gwp-on-gw",
			Validate:  EmptySecurityContextValidator(),
		},
		{
			Name:      "agentgateway-infrastructure with AgentgatewayParameters",
			InputFile: "agentgateway-infrastructure",
		},
		{
			Name:      "agentgateway-controller-but-custom-gatewayclass",
			InputFile: "agentgateway-controller-but-custom-gatewayclass",
		},
		{
			Name:      "agentgateway-params-primary",
			InputFile: "agentgateway-params-primary",
		},
		{
			Name:      "agentgateway with full image override",
			InputFile: "agentgateway-image-override",
		},
		{
			Name:      "agentgateway with env vars",
			InputFile: "agentgateway-env",
		},
		{
			Name:      "agentgateway with shutdown configuration",
			InputFile: "agentgateway-shutdown",
		},
		{
			Name:      "agentgateway with Istio configuration",
			InputFile: "agentgateway-istio",
		},
		{
			Name:      "agentgateway with logging format json",
			InputFile: "agentgateway-logging-format",
		},
		{
			Name:      "agentgateway yaml injection",
			InputFile: "agentgateway-yaml-injection",
		},
		{
			Name:      "agentgateway rawConfig with typed config conflict",
			InputFile: "agentgateway-rawconfig-typed-conflict",
			Validate: func(t *testing.T, outputYaml string) {
				t.Helper()
				assert.Contains(t, outputYaml, "format: text",
					"typed logging.format: text should take precedence over rawConfig's json")
				assert.NotContains(t, outputYaml, "format: json",
					"rawConfig's logging.format: json should be overridden by typed config")
				assert.Contains(t, outputYaml, "jaeger:4317",
					"tracing config from rawConfig should be merged in")
			},
		},
		{
			Name:      "agentgateway rawConfig with binds for direct response",
			InputFile: "agentgateway-rawconfig-binds",
			Validate: func(t *testing.T, outputYaml string) {
				t.Helper()
				assert.Contains(t, outputYaml, "  config.yaml: |\n    binds:\n",
					"binds config should be present in ConfigMap as a top-level config.yaml key")
				assert.Contains(t, outputYaml, "port: 3000",
					"binds port 3000 should be present")
			},
		},
		{
			Name:      "agentgateway with repository only image override",
			InputFile: "agentgateway-image-repo-only",
			Validate: func(t *testing.T, outputYaml string) {
				t.Helper()
				assert.NotContains(t, outputYaml, "imagePullPolicy:",
					"output YAML should not contain imagePullPolicy, allowing k8s to look at the tag to decide")
			},
		},
		{
			// Test merging GWC and GW AgentgatewayParameters.
			Name:      "agentgateway both GWC and GW have parametersRef",
			InputFile: "agentgateway-both-gwc-and-gw-have-params",
		},
		{
			Name:      "agentgateway strategic-merge-patch tests",
			InputFile: "agentgateway-strategic-merge-patch",
			Validate: func(t *testing.T, outputYaml string) {
				t.Helper()
				// Deployment overlay metadata applied
				assert.Contains(t, outputYaml, "deployment-overlay-annotation: from-overlay",
					"deployment annotation from overlay should be present")
				assert.Contains(t, outputYaml, "deployment-overlay-label1: from-overlay",
					"deployment label from overlay should be present")

				// $patch: delete on env var RUST_LOG
				assert.NotContains(t, outputYaml, "RUST_LOG",
					"RUST_LOG env var should be deleted via $patch: delete")

				// $patch: replace on volumes - only custom-config volume in volumes list
				// (volumeMounts is a separate list that still has the original mounts)
				assert.Contains(t, outputYaml, "name: my-custom-config",
					"custom configmap from $patch: replace should be present")
				// Verify only one volume exists (the custom one) by checking volumes section structure
				assert.Contains(t, outputYaml, "volumes:\n      - configMap:\n          name: my-custom-config\n        name: custom-config\nstatus:",
					"volumes should be replaced with only custom-config")

				// $patch: replace on service ports
				assert.Contains(t, outputYaml, "port: 80",
					"service port 80 from $patch: replace should be present")
				assert.Contains(t, outputYaml, "port: 443",
					"service port 443 from $patch: replace should be present")
				// The original Gateway listener port 8080 becomes targetPort, not port
				assert.NotContains(t, outputYaml, "port: 8080\n",
					"default port 8080 should be replaced (only exists as targetPort now)")

				// $setElementOrder/args - args reordered
				assert.Contains(t, outputYaml, "- /config/config.yaml\n        - -f",
					"args should be reordered via $setElementOrder")

				// Service overlay annotation
				assert.Contains(t, outputYaml, "service-overlay-annotation: from-overlay",
					"service annotation from overlay should be present")

				// Label nulled to empty string
				assert.Contains(t, outputYaml, `app.kubernetes.io/managed-by: ""`,
					"label should be nulled to empty string")

				// Volume mount added via merge
				assert.Contains(t, outputYaml, "mountPath: /etc/custom-config",
					"custom volumeMount should be added via merge")
			},
		},
		{
			Name:      "agentgateway AGWP with pod scheduling fields",
			InputFile: "agentgateway-agwp-pod-scheduling",
		},
		{
			Name:      "agentgateway with static IP address via overlay",
			InputFile: "agentgateway-loadbalancer-static-ip",
		},
		{
			Name:      "agentgateway GKE with subsetting and external static IP",
			InputFile: "agentgateway-gke-subsetting-static-ip",
		},
		{
			Name:      "agentgateway with PodDisruptionBudget overlay",
			InputFile: "agentgateway-pdb-overlay",
			Validate: func(t *testing.T, outputYaml string) {
				t.Helper()
				assert.Contains(t, outputYaml, "kind: PodDisruptionBudget",
					"PDB should be created when podDisruptionBudget overlay is specified")
				assert.Contains(t, outputYaml, "pdb-label: from-overlay",
					"PDB should have label from overlay")
				assert.Contains(t, outputYaml, "pdb-annotation: from-overlay",
					"PDB should have annotation from overlay")
				assert.Contains(t, outputYaml, "minAvailable: 1",
					"PDB should have minAvailable from overlay spec")
			},
		},
		{
			Name:      "agentgateway with HorizontalPodAutoscaler overlay",
			InputFile: "agentgateway-hpa-overlay",
			Validate: func(t *testing.T, outputYaml string) {
				t.Helper()
				assert.Contains(t, outputYaml, "kind: HorizontalPodAutoscaler",
					"HPA should be created when horizontalPodAutoscaler overlay is specified")
				assert.Contains(t, outputYaml, "hpa-label: from-overlay",
					"HPA should have label from overlay")
				assert.Contains(t, outputYaml, "hpa-annotation: from-overlay",
					"HPA should have annotation from overlay")
				assert.Contains(t, outputYaml, "minReplicas: 2",
					"HPA should have minReplicas from overlay spec")
				assert.Contains(t, outputYaml, "maxReplicas: 10",
					"HPA should have maxReplicas from overlay spec")
				assert.Contains(t, outputYaml, "averageUtilization: 80",
					"HPA should have CPU utilization target from overlay spec")
			},
		},

		// TLS test cases
		{
			Name:                        "agentgateway with TLS enabled",
			InputFile:                   "agentgateway-tls",
			HelmValuesGeneratorOverride: tlsOverride(caCertPath),
		},
		{
			// Custom configmap name via AgentgatewayParameters deployment overlay:
			Name:      "agentgateway with custom configmap name via overlay",
			InputFile: "agentgateway-custom-configmap",
		},
		{
			Name:      "agentgateway with Gateway.spec.addresses",
			InputFile: "agentgateway-gateway-addresses",
		},
		{
			Name:      "agentgateway with name exactly 63 characters",
			InputFile: "agentgateway-long-gateway-name-exactly-63-chars",
		},
		{
			Name:      "agentgateway with name over 63 characters",
			InputFile: "agentgateway-long-gateway-name-over-63-chars",
		},
	}

	tester := DeployerTester{
		AgwControllerName: wellknown.DefaultAgwControllerName,
		AgwClassName:      wellknown.DefaultAgwClassName,
	}

	dir := fsutils.MustGetThisDir()
	scheme := schemes.GatewayScheme()
	crdDir := filepath.Join(testutils.GitRootDirectory(), testutils.AgwCRDPath)

	VerifyAllYAMLFilesReferenced(t, filepath.Join(dir, "testdata"), tests)

	for _, tt := range tests {
		t.Run(tt.Name, func(t *testing.T) {
			fakeClient := fake.NewClient(t, tester.GetObjects(t, tt, scheme, dir, crdDir)...)
			tester.RunHelmChartTest(t, tt, scheme, dir, crdDir, fakeClient)
		})
	}
}
