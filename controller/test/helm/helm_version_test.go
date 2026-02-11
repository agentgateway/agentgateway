package helm

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/stretchr/testify/require"
)

// TestImageTagVPrefix verifies that image tags always have a 'v' prefix,
// regardless of whether AppVersion or explicit image.tag values include one.
func TestImageTagVPrefix(t *testing.T) {
	charts := []struct {
		name             string
		path             string
		repository       string
		hasDefaultEnvTag bool // Whether the chart sets KGW_DEFAULT_IMAGE_TAG
	}{
		{
			name:             "agentgateway",
			path:             filepath.Join("..", "..", "install", "helm", "agentgateway"),
			repository:       "controller",
			hasDefaultEnvTag: false,
		},
	}

	testCases := []struct {
		name           string
		setValues      []string
		expectedTag    string
		expectedEnvTag string // Expected KGW_DEFAULT_IMAGE_TAG value (empty = same as expectedTag)
	}{
		{
			name:        "default AppVersion without v prefix gets v added",
			setValues:   nil, // Uses Chart.AppVersion which is "0.0.1"
			expectedTag: "v0.0.1",
		},
		{
			name:        "explicit tag with v prefix is not doubled",
			setValues:   []string{"image.tag=v2.0.0"},
			expectedTag: "v2.0.0",
		},
		{
			name:        "explicit tag without v prefix gets v added",
			setValues:   []string{"image.tag=1.2.3"},
			expectedTag: "v1.2.3",
		},
		{
			name:           "controller-specific tag with v prefix is not doubled",
			setValues:      []string{"controller.image.tag=v3.0.0"},
			expectedTag:    "v3.0.0",
			expectedEnvTag: "v0.0.1", // KGW_DEFAULT_IMAGE_TAG falls back to AppVersion
		},
		{
			name:           "controller-specific tag without v prefix gets v added",
			setValues:      []string{"controller.image.tag=3.0.0"},
			expectedTag:    "v3.0.0",
			expectedEnvTag: "v0.0.1", // KGW_DEFAULT_IMAGE_TAG falls back to AppVersion
		},
		{
			name:        "latest tag is not modified",
			setValues:   []string{"image.tag=latest"},
			expectedTag: "latest",
		},
		{
			name:        "dev tag is not modified",
			setValues:   []string{"image.tag=dev"},
			expectedTag: "dev",
		},
	}

	for _, chart := range charts {
		for _, tc := range testCases {
			testName := chart.name + "/" + tc.name
			t.Run(testName, func(t *testing.T) {
				absHelmChartPath, err := filepath.Abs(chart.path)
				require.NoError(t, err, "failed to get absolute path for helm chart")

				_, err = os.Stat(absHelmChartPath)
				require.NoError(t, err, "helm chart not found at %s", absHelmChartPath)

				args := []string{"template", "test-release", absHelmChartPath, "--namespace", "default"}
				for _, setValue := range tc.setValues {
					args = append(args, "--set", setValue)
				}

				helmCmd := exec.Command("helm", args...)
				var output bytes.Buffer
				var stderr bytes.Buffer
				helmCmd.Stdout = &output
				helmCmd.Stderr = &stderr

				err = helmCmd.Run()
				require.NoError(t, err, "helm template failed: %s", stderr.String())

				outputStr := output.String()

				expectedImageSuffix := chart.repository + ":" + tc.expectedTag
				if !strings.Contains(outputStr, expectedImageSuffix) {
					t.Errorf("expected image tag %q not found in output.\nLooking for: %s\nOutput snippet:\n%s",
						tc.expectedTag, expectedImageSuffix, extractImageLines(outputStr))
				}

				if chart.hasDefaultEnvTag {
					envTag := tc.expectedTag
					if tc.expectedEnvTag != "" {
						envTag = tc.expectedEnvTag
					}
					expectedEnvValue := "value: " + envTag
					if !strings.Contains(outputStr, expectedEnvValue) {
						t.Errorf("expected KGW_DEFAULT_IMAGE_TAG value %q not found in output", envTag)
					}
				}
			})
		}
	}
}

// extractImageLines extracts lines containing "image:" from the output for debugging
func extractImageLines(output string) string {
	var lines []string
	for line := range strings.SplitSeq(output, "\n") {
		if strings.Contains(line, "image:") {
			lines = append(lines, strings.TrimSpace(line))
		}
	}
	return strings.Join(lines, "\n")
}

// TestHelmChartTemplate tests helm template output for both kgateway and agentgateway charts
// with different values configurations.
func TestHelmChartTemplate(t *testing.T) {
	charts := []string{"agentgateway"}

	valuesCases := []struct {
		name       string
		valuesYAML string
	}{
		{
			name:       "default",
			valuesYAML: "",
		},
		{
			name: "xds-tls-enabled",
			valuesYAML: `controller:
  xds:
    tls:
      enabled: true
`,
		},
		{
			name: "pdb-min-available",
			valuesYAML: `controller:
  podDisruptionBudget:
    minAvailable: 1
`,
		},
		{
			name: "pdb-max-unavailable",
			valuesYAML: `controller:
  podDisruptionBudget:
    maxUnavailable: 25%
`,
		},
		{
			name: "service-full-config",
			valuesYAML: `controller:
  service:
    type: LoadBalancer
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-type: nlb
      service.beta.kubernetes.io/aws-load-balancer-internal: "true"
    extraLabels:
      custom-label: custom-value
      environment: test
    clusterIP: ""
    clusterIPs:
      - 10.96.0.100
    externalIPs:
      - 203.0.113.10
      - 203.0.113.11
    loadBalancerIP: 198.51.100.1
    loadBalancerSourceRanges:
      - 10.0.0.0/8
      - 192.168.0.0/16
    loadBalancerClass: service.k8s.aws/nlb
    externalTrafficPolicy: Local
    internalTrafficPolicy: Cluster
    healthCheckNodePort: 32100
    sessionAffinity: ClientIP
    sessionAffinityConfig:
      clientIP:
        timeoutSeconds: 10800
    ipFamilies:
      - IPv4
    ipFamilyPolicy: SingleStack
    publishNotReadyAddresses: true
    allocateLoadBalancerNodePorts: false
    trafficDistribution: PreferClose
`,
		},
		{
			name: "hpa-and-vpa",
			valuesYAML: `controller:
  horizontalPodAutoscaler:
    minReplicas: 1
    maxReplicas: 5
    metrics:
      - type: Resource
        resource:
          name: cpu
          target:
            type: Utilization
            averageUtilization: 80
  verticalPodAutoscaler:
    updatePolicy:
      updateMode: Auto
    resourcePolicy:
      containerPolicies:
        - containerName: "*"
          minAllowed:
            cpu: 100m
            memory: 128Mi
`,
		},
		{
			name: "priority-class-name",
			valuesYAML: `controller:
  priorityClassName: system-cluster-critical
`,
		},
		{
			name: "additional-labels",
			valuesYAML: `commonLabels:
    extra-label-key: extra-label-value
    another-label: "true"
`,
		},
	}

	for _, chart := range charts {
		for _, vc := range valuesCases {
			testName := chart + "/" + vc.name
			t.Run(testName, func(t *testing.T) {
				helmChartPath := filepath.Join("..", "..", "install", "helm", chart)
				absHelmChartPath, err := filepath.Abs(helmChartPath)
				require.NoError(t, err, "failed to get absolute path for helm chart")

				_, err = os.Stat(absHelmChartPath)
				require.NoError(t, err, "helm chart not found at %s", absHelmChartPath)

				// Build helm template command args
				// Explicitly set namespace to avoid picking up the current kubectl context's namespace
				args := []string{"template", "test-release", absHelmChartPath, "--namespace", "default"}

				// If we have custom values, write them to a temp file
				if vc.valuesYAML != "" {
					valuesFile, err := os.CreateTemp("", "values-*.yaml")
					require.NoError(t, err, "failed to create temp values file")
					defer os.Remove(valuesFile.Name())

					_, err = valuesFile.WriteString(vc.valuesYAML)
					require.NoError(t, err, "failed to write values file")
					err = valuesFile.Close()
					require.NoError(t, err, "failed to close values file")

					args = append(args, "-f", valuesFile.Name())
				}

				helmCmd := exec.Command("helm", args...)
				var output bytes.Buffer
				var stderr bytes.Buffer
				helmCmd.Stdout = &output
				helmCmd.Stderr = &stderr

				err = helmCmd.Run()
				require.NoError(t, err, "helm template failed: %s", stderr.String())

				got := output.Bytes()

				// Golden file path: testdata/<chart>/<values-case>.golden
				goldenDir := filepath.Join("testdata", chart)
				goldenFile := filepath.Join(goldenDir, vc.name+".golden")

				absGoldenFile, err := filepath.Abs(goldenFile)
				require.NoError(t, err, "failed to get absolute path for golden file")

				refreshGolden := strings.ToLower(os.Getenv("REFRESH_GOLDEN"))
				if refreshGolden == "true" || refreshGolden == "1" {
					t.Log("REFRESH_GOLDEN is set, writing golden file", absGoldenFile)
					err = os.MkdirAll(goldenDir, 0o755)
					require.NoError(t, err, "failed to create testdata directory")

					err = os.WriteFile(absGoldenFile, got, 0o644) //nolint:gosec // G306: Golden test file can be readable
					require.NoError(t, err, "failed to write golden file")
					return
				}

				want, err := os.ReadFile(absGoldenFile)
				require.NoError(t, err, "failed to read golden file %s; run with REFRESH_GOLDEN=true to generate", absGoldenFile)

				diff := cmp.Diff(string(want), string(got))
				if diff != "" {
					t.Errorf("helm template output differs from golden file (-want +got):\n%s\n\nTo refresh: REFRESH_GOLDEN=true go test ./test/helm", diff)
				}
			})
		}
	}
}
