package helm

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/stretchr/testify/require"
)

func renderStandaloneChart(t *testing.T, valuesYAML string) (string, string, error) {
	t.Helper()
	chartPath := filepath.Join("..", "..", "install", "helm", "agentgateway-standalone")
	absChartPath, err := filepath.Abs(chartPath)
	require.NoError(t, err)

	args := []string{"template", "test-release", absChartPath, "--namespace", "default"}
	if valuesYAML != "" {
		valuesFile, err := os.CreateTemp("", "standalone-values-*.yaml")
		require.NoError(t, err)
		t.Cleanup(func() {
			_ = os.Remove(valuesFile.Name())
		})
		_, err = valuesFile.WriteString(valuesYAML)
		require.NoError(t, err)
		require.NoError(t, valuesFile.Close())
		args = append(args, "-f", valuesFile.Name())
	}

	cmd := helmCommand(t, args...)
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err = cmd.Run()
	return normalizeStandaloneHelmOutput(stdout.String()), stderr.String(), err
}

func normalizeStandaloneHelmOutput(out string) string {
	return strings.ReplaceAll(out, "\n\n---\n# Source: agentgateway-standalone/", "\n---\n# Source: agentgateway-standalone/")
}

func TestStandaloneChartGoldenTemplate(t *testing.T) {
	testCases := []struct {
		name       string
		valuesYAML string
	}{
		{
			name:       "default",
			valuesYAML: "",
		},
		{
			name: "service-full-config",
			valuesYAML: `gateway:
  service:
    type: LoadBalancer
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-scheme: internet-facing
    extraLabels:
      plane: gateway
    clusterIPs:
    - 10.96.0.20
    externalIPs:
    - 203.0.113.20
    - 203.0.113.21
    loadBalancerIP: 198.51.100.20
    loadBalancerSourceRanges:
    - 192.168.0.0/16
    loadBalancerClass: service.k8s.aws/nlb
    externalTrafficPolicy: Local
    internalTrafficPolicy: Cluster
    healthCheckNodePort: 32101
    sessionAffinity: ClientIP
    sessionAffinityConfig:
      clientIP:
        timeoutSeconds: 3600
    ipFamilies:
    - IPv4
    ipFamilyPolicy: SingleStack
    publishNotReadyAddresses: true
    allocateLoadBalancerNodePorts: false
    trafficDistribution: PreferClose
    ports:
    - name: public-http
      port: 80
      targetPort: 8080
      protocol: TCP
    - name: public-https
      port: 443
      targetPort: 8443
      protocol: TCP
  extraServices:
  - name: public-3000
    type: LoadBalancer
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-type: nlb
    extraLabels:
      listener: public-3000
    loadBalancerClass: service.k8s.aws/nlb
    externalTrafficPolicy: Local
    ports:
    - name: listener-3000
      port: 3000
      targetPort: 3000
      protocol: TCP
`,
		},
		{
			name: "workload-overrides",
			valuesYAML: `revisionHistoryLimit: 3
resources:
  requests:
    cpu: 250m
    memory: 256Mi
  limits:
    cpu: "1"
    memory: 1Gi
nodeSelector:
  kubernetes.io/os: linux
dnsConfig:
  options:
  - name: ndots
    value: "3"
tolerations:
- key: dedicated
  operator: Equal
  value: agentgateway
  effect: NoSchedule
affinity:
  podAntiAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
    - labelSelector:
        matchLabels:
          app.kubernetes.io/component: standalone
      topologyKey: kubernetes.io/hostname
extraEnv:
- name: LOG_FORMAT
  value: json
- name: API_TOKEN
  valueFrom:
    secretKeyRef:
      name: agw-secret
      key: token
extraVolumes:
- name: plugin-cache
  emptyDir: {}
extraVolumeMounts:
- name: plugin-cache
  mountPath: /var/lib/agentgateway/plugins
`,
		},
		{
			name: "monitoring-full-config",
			valuesYAML: `monitoring:
  enabled: true
  annotations:
    example.com/note: monitoring
  extraLabels:
    plane: monitoring
  podMonitor:
    enabled: true
    interval: 30s
`,
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			got, stderr, err := renderStandaloneChart(t, tc.valuesYAML)
			require.NoError(t, err, "helm template failed: %s", stderr)

			goldenDir := filepath.Join("testdata", "agentgateway-standalone")
			goldenFile := filepath.Join(goldenDir, tc.name+".golden")
			absGoldenFile, err := filepath.Abs(goldenFile)
			require.NoError(t, err)

			refreshGolden := strings.ToLower(os.Getenv("REFRESH_GOLDEN"))
			if refreshGolden == "true" || refreshGolden == "1" {
				require.NoError(t, os.MkdirAll(goldenDir, 0o755))
				require.NoError(t, os.WriteFile(absGoldenFile, []byte(got), 0o644)) //nolint:gosec // G306: Golden test file can be readable
				return
			}

			want, err := os.ReadFile(absGoldenFile)
			require.NoError(t, err, "failed to read golden file %s; run with REFRESH_GOLDEN=true to generate", absGoldenFile)

			if diff := cmp.Diff(string(want), got); diff != "" {
				t.Errorf("helm template output differs from golden file (-want +got):\n%s\n\nTo refresh: REFRESH_GOLDEN=true go test ./test/helm -run TestStandaloneChartGoldenTemplate", diff)
			}
		})
	}
}

func TestStandaloneChartDefaultRender(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, "")
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.NotContains(t, out, "kind: PersistentVolumeClaim")
	require.Contains(t, out, "name: test-release-config")
	require.Contains(t, out, "namespace: default")
	require.NotContains(t, out, "database:")
	require.Contains(t, out, "storage:\n        mode: file")
	require.Contains(t, out, "gateways:")
	require.Contains(t, out, "default:")
	require.Contains(t, out, "port: 4000")
	require.NotContains(t, out, "binds:")
	require.NotContains(t, out, "strategy:\n    type: Recreate")
	require.Contains(t, out, "kind: Service\nmetadata:\n  name: test-release")
	require.NotContains(t, out, "name: test-release-admin")
	require.NotContains(t, out, "adminAddr:")
	require.Contains(t, out, "- name: \"http\"\n    protocol: \"TCP\"\n    port: 80\n    targetPort: 4000")
	require.NotContains(t, out, "port: 443")
	require.NotContains(t, out, "port: 3000")
	require.NotContains(t, out, "net.ipv4.ip_unprivileged_port_start")
	require.NotContains(t, out, "runAsUser: 10101")
	require.NotContains(t, out, "runAsGroup: 10101")
	require.NotContains(t, out, "fsGroup: 10101")
	require.NotContains(t, out, "fsGroupChangePolicy: OnRootMismatch")
	require.NotContains(t, out, "- name: config-bootstrap")
	require.NotContains(t, out, "image: \"docker.io/library/busybox:1.36\"")
	require.Contains(t, out, "mountPath: /config\n          readOnly: true")
	require.Contains(t, out, "- name: config\n        configMap:\n          name: test-release-config")
	require.Contains(t, out, "allowPrivilegeEscalation: false")
	require.Contains(t, out, "readOnlyRootFilesystem: true")
	require.Contains(t, out, "readinessProbe:\n          httpGet:\n            path: /healthz/ready\n            port: 15021\n          periodSeconds: 10")
	require.Contains(t, out, "startupProbe:\n          failureThreshold: 60\n          httpGet:\n            path: /healthz/ready\n            port: 15021\n          periodSeconds: 1\n          successThreshold: 1\n          timeoutSeconds: 2")
	require.NotContains(t, out, "name: AGENTGATEWAY_ENV")
	require.Contains(t, out, "name: OIDC_COOKIE_SECRET")
	require.Contains(t, out, "name: test-release-oidc\n              key: OIDC_COOKIE_SECRET\n              optional: true")
	require.NotContains(t, out, `"helm.sh/hook": test`)
	require.NotContains(t, out, "curlimages/curl")
}

func TestStandaloneChartConfiguredOIDCCookieSecretIsRequired(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `oidc:
  cookieSecretName: platform-oidc
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "name: platform-oidc\n              key: OIDC_COOKIE_SECRET\n              optional: false")
	require.NotContains(t, out, "name: test-release-oidc")
}

func TestStandaloneChartInlineConfig(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `config:
  gateways:
    default:
      port: 3000
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "gateways:")
	require.Contains(t, out, "port: 3000")
}

func TestStandaloneChartDatabaseModeAllowsReplicas(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `replicaCount: 3
mode: database
database:
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "replicas: 3")
	require.Contains(t, out, "storage:\n        mode: hybrid")
	require.Contains(t, out, "url: postgres://agw:secret@postgres.default.svc:5432/agw")
	require.NotContains(t, out, "kind: PersistentVolumeClaim")
	require.NotContains(t, out, "- name: config-bootstrap")
	require.Contains(t, out, "mountPath: /config\n          readOnly: true")
}

func TestStandaloneChartPerImageRegistries(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `image:
  registry: registry.one.example.com
  repository: platform/agentgateway
  tag: dev
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, `image: "registry.one.example.com/platform/agentgateway:dev"`)
}

func TestStandaloneChartStringImage(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `image: localhost:5000/agentgateway:1784825828
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, `image: "localhost:5000/agentgateway:1784825828"`)
	require.Contains(t, out, "imagePullPolicy: IfNotPresent")
}

func TestStandaloneChartRejectsUnknownMode(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `mode: other
`)
	require.Error(t, err)
	require.Contains(t, stderr, "mode must be one of: readonly, database")
}

func TestStandaloneChartRejectsDatabaseModeWithoutPostgres(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `mode: database
`)
	require.Error(t, err)
	require.Contains(t, stderr, "mode=database requires database.postgres.url")
}

func TestStandaloneChartRejectsDatabaseModeWithNonPostgresURL(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `mode: database
database:
  postgres:
    url: sqlite:///config/data.db
`)
	require.Error(t, err)
	require.Contains(t, stderr, "to start with postgres:// or postgresql://")
}

func TestStandaloneChartRejectsPostgresOutsideDatabaseMode(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `mode: readonly
database:
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
`)
	require.Error(t, err)
	require.Contains(t, stderr, `database.postgres.url is only supported when mode=database (got mode "readonly")`)
}

func TestStandaloneChartReadonlyAllowsReplicas(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `replicaCount: 2
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "replicas: 2")
	require.NotContains(t, out, "kind: PersistentVolumeClaim")
}

func TestStandaloneChartCustomGatewayPorts(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `gateway:
  service:
    ports:
    - name: listener-3000
      port: 3000
      targetPort: 3000
      protocol: TCP
    - name: listener-4000
      port: 4000
      targetPort: 4000
      protocol: TCP
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "name: \"listener-3000\"")
	require.Contains(t, out, "port: 3000")
	require.Contains(t, out, "name: \"listener-4000\"")
	require.Contains(t, out, "port: 4000")
	require.NotContains(t, out, "name: http")
	require.NotContains(t, out, "\n    port: 80\n")
}

func TestStandaloneChartGatewayExtraServices(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `gateway:
  extraServices:
  - name: private-listener
    type: ClusterIP
    annotations:
      networking.example.com/scope: private
    extraLabels:
      listener: private
    ports:
    - name: private
      port: 3000
      targetPort: 3000
      protocol: TCP
  - name: public-listener
    type: LoadBalancer
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-type: nlb
    extraLabels:
      listener: public
    loadBalancerClass: service.k8s.aws/nlb
    externalTrafficPolicy: Local
    loadBalancerSourceRanges:
    - 10.0.0.0/8
    ports:
    - name: public
      port: 80
      targetPort: 8080
      protocol: TCP
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "name: test-release-private-listener")
	require.Contains(t, out, "name: test-release-public-listener")
	require.Contains(t, out, "networking.example.com/scope: private")
	require.Contains(t, out, "service.beta.kubernetes.io/aws-load-balancer-type: nlb")
	require.Contains(t, out, "listener: private")
	require.Contains(t, out, "listener: public")
	require.Contains(t, out, "loadBalancerClass: service.k8s.aws/nlb")
	require.Contains(t, out, "externalTrafficPolicy: Local")
	require.Contains(t, out, "loadBalancerSourceRanges:\n    - 10.0.0.0/8")
	require.Contains(t, out, "name: \"private\"")
	require.Contains(t, out, "port: 3000")
	require.Contains(t, out, "name: \"public\"")
	require.Contains(t, out, "targetPort: 8080")
}

func TestStandaloneChartRejectsGatewayExtraServiceWithoutName(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `gateway:
  extraServices:
  - ports:
    - name: listener
      port: 3000
      targetPort: 3000
`)
	require.Error(t, err)
	require.Contains(t, stderr, "gateway.extraServices[].name is required")
}

func TestStandaloneChartRejectsGatewayExtraServiceWithoutPorts(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `gateway:
  extraServices:
  - name: listener
`)
	require.Error(t, err)
	require.Contains(t, stderr, "gateway.extraServices[listener].ports must contain at least one port")
}

func TestStandaloneChartServiceFullConfig(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `gateway:
  service:
    type: LoadBalancer
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-scheme: internet-facing
    extraLabels:
      plane: gateway
    clusterIPs:
    - 10.96.0.20
    externalIPs:
    - 203.0.113.20
    loadBalancerIP: 198.51.100.20
    loadBalancerSourceRanges:
    - 192.168.0.0/16
    loadBalancerClass: service.k8s.aws/nlb
    externalTrafficPolicy: Local
    internalTrafficPolicy: Cluster
    healthCheckNodePort: 32101
    sessionAffinity: ClientIP
    sessionAffinityConfig:
      clientIP:
        timeoutSeconds: 3600
    ipFamilies:
    - IPv4
    ipFamilyPolicy: SingleStack
    publishNotReadyAddresses: true
    allocateLoadBalancerNodePorts: false
    trafficDistribution: PreferClose
    ports:
    - name: public-http
      port: 80
      targetPort: 8080
      protocol: TCP
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "plane: gateway")
	require.Contains(t, out, "clusterIPs:\n    - 10.96.0.20")
	require.Contains(t, out, "loadBalancerIP: 198.51.100.20")
	require.Contains(t, out, "healthCheckNodePort: 32101")
	require.Contains(t, out, "allocateLoadBalancerNodePorts: false")
	require.Contains(t, out, "trafficDistribution: PreferClose")
	require.Contains(t, out, "name: \"public-http\"")
	require.Contains(t, out, "targetPort: 8080")
}

func TestStandaloneChartWorkloadOverrides(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `resources:
  requests:
    cpu: 250m
    memory: 256Mi
  limits:
    cpu: "1"
    memory: 1Gi
nodeSelector:
  kubernetes.io/os: linux
tolerations:
- key: dedicated
  operator: Equal
  value: agentgateway
  effect: NoSchedule
affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
    - weight: 100
      podAffinityTerm:
        labelSelector:
          matchLabels:
            app.kubernetes.io/component: standalone
        topologyKey: kubernetes.io/hostname
extraEnv:
- name: LOG_FORMAT
  value: json
- name: API_TOKEN
  valueFrom:
    secretKeyRef:
      name: agw-secret
      key: token
extraVolumes:
- name: plugin-cache
  emptyDir: {}
extraVolumeMounts:
- name: plugin-cache
  mountPath: /var/lib/agentgateway/plugins
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "cpu: 250m")
	require.Contains(t, out, "memory: 1Gi")
	require.Contains(t, out, "kubernetes.io/os: linux")
	require.Contains(t, out, "key: dedicated")
	require.Contains(t, out, "podAntiAffinity:")
	require.Contains(t, out, "name: LOG_FORMAT")
	require.Contains(t, out, "secretKeyRef:")
	require.Contains(t, out, "name: plugin-cache")
	require.Contains(t, out, "mountPath: /var/lib/agentgateway/plugins")
}

func TestStandaloneChartExtraContainers(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `extraContainers:
- name: httpbin
  image: kennethreitz/httpbin
  ports:
    - containerPort: 80
      name: httpbin
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "name: httpbin")
	require.Contains(t, out, "image: kennethreitz/httpbin")
	require.Contains(t, out, "containerPort: 80")
}

func TestStandaloneChartMonitoringDisabledByDefault(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, "")
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.NotContains(t, out, "kind: PodMonitor")
	require.NotContains(t, out, "name: metrics")
	require.Contains(t, out, "prometheus.io/scrape")
}

func TestStandaloneChartMonitoringEnabled(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `monitoring:
  enabled: true
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "prometheus.io/scrape:")
	require.Contains(t, out, "kind: PodMonitor")
	require.Contains(t, out, "name: agentgateway-standalone")
	require.Contains(t, out, "- name: metrics\n          containerPort: 15020")
	require.Contains(t, out, "prometheus.io/port: \"15020\"")
	require.Contains(t, out, "prometheus.io/path: /metrics")
	require.Contains(t, out, "- port: metrics\n    path: /metrics\n    interval: 15s")
}

func TestStandaloneChartMonitoringPodMonitorDisabled(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `monitoring:
  enabled: true
  podMonitor:
    enabled: false
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.NotContains(t, out, "kind: PodMonitor")
	require.Contains(t, out, "prometheus.io/scrape:")
	require.Contains(t, out, "- name: metrics\n          containerPort: 15020")
}

func TestStandaloneChartMonitoringFullConfig(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `monitoring:
  enabled: true
  annotations:
    example.com/note: monitoring
  extraLabels:
    plane: monitoring
  podMonitor:
    enabled: true
    interval: 30s
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "plane: monitoring")
	require.Contains(t, out, "example.com/note: monitoring")
	require.Contains(t, out, "containerPort: 15020")
	require.Contains(t, out, "prometheus.io/port: \"15020\"")
	require.Contains(t, out, "interval: 30s")
}

func TestStandaloneChartPodAnnotationsMergeWithMonitoringPort(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `podAnnotations:
  team: platform
monitoring:
  enabled: true
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "team: platform")
	require.Contains(t, out, "prometheus.io/path: /metrics")
	require.Contains(t, out, "prometheus.io/port: \"15020\"")
}

func TestStandaloneChartPodAnnotationsOverridesMonitoringPort(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `podAnnotations:
  prometheus.io/port: "9999"
monitoring:
  enabled: false
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "prometheus.io/port: \"9999\"")
}

func TestStandaloneChartMonitoringRemovePrometheusAnnotations(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `podAnnotations: {}
monitoring:
  enabled: true
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.NotContains(t, out, "prometheus.io:")
}
