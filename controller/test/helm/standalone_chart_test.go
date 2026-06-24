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

	cmd := exec.Command("helm", args...)
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
			valuesYAML: `admin:
  service:
    type: LoadBalancer
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-type: nlb
    extraLabels:
      plane: admin
    clusterIP: ""
    clusterIPs:
    - 10.96.0.10
    externalIPs:
    - 203.0.113.10
    loadBalancerIP: 198.51.100.10
    loadBalancerSourceRanges:
    - 10.0.0.0/8
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
gateway:
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
			valuesYAML: `resources:
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
	require.Contains(t, out, "kind: PersistentVolumeClaim")
	require.Contains(t, out, "name: agentgateway-standalone-config")
	require.Contains(t, out, "namespace: agentgateway-system")
	require.Contains(t, out, "sqlite:///config/data.db")
	require.Contains(t, out, "binds:")
	require.Contains(t, out, "port: 8080")
	require.Contains(t, out, "port: 8443")
	require.Contains(t, out, "listeners: []")
	require.Contains(t, out, "strategy:\n    type: Recreate")
	require.Contains(t, out, "name: agentgateway-standalone-admin")
	require.Contains(t, out, "name: agentgateway-standalone-gateway")
	require.Contains(t, out, "- name: \"http\"\n    protocol: \"TCP\"\n    port: 80\n    targetPort: 8080")
	require.Contains(t, out, "- name: \"https\"\n    protocol: \"TCP\"\n    port: 443\n    targetPort: 8443")
	require.Contains(t, out, "- name: \"mcp\"\n    protocol: \"TCP\"\n    port: 3000\n    targetPort: 3000")
	require.Contains(t, out, "- name: \"llm\"\n    protocol: \"TCP\"\n    port: 4000\n    targetPort: 4000")
	require.NotContains(t, out, "net.ipv4.ip_unprivileged_port_start")
	require.NotContains(t, out, "runAsUser: 10101")
	require.NotContains(t, out, "runAsGroup: 10101")
	require.NotContains(t, out, "fsGroup: 10101")
	require.NotContains(t, out, "fsGroupChangePolicy: OnRootMismatch")
	require.Contains(t, out, "- name: config-bootstrap")
	require.Contains(t, out, "image: \"docker.io/library/busybox:1.36\"")
	require.Contains(t, out, `if [ "false" = "true" ] || [ ! -f /config/config.yaml ]; then`)
	require.Contains(t, out, `tmp="$(mktemp /config/config.yaml.XXXXXX)"`)
	require.Contains(t, out, "mv \"$tmp\" /config/config.yaml")
	require.Contains(t, out, "allowPrivilegeEscalation: false")
	require.Contains(t, out, "readOnlyRootFilesystem: true")
	require.NotContains(t, out, "name: AGENTGATEWAY_ENV")
	require.NotContains(t, out, `"helm.sh/hook": test`)
	require.NotContains(t, out, "curlimages/curl")
}

func TestStandaloneChartExistingClaim(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `persistence:
  existingClaim: agw-config
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.NotContains(t, out, "kind: PersistentVolumeClaim")
	require.Contains(t, out, "claimName: agw-config")
}

func TestStandaloneChartPersistenceDisabledUsesEmptyDir(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `persistence:
  enabled: false
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.NotContains(t, out, "kind: PersistentVolumeClaim")
	require.Contains(t, out, "- name: config\n        emptyDir: {}")
	require.NotContains(t, out, "- name: config\n        persistentVolumeClaim:")
}

func TestStandaloneChartInlineConfig(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `config:
  config:
    adminAddr: 0.0.0.0:15000
  binds:
  - port: 8080
    listeners: []
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "adminAddr: 0.0.0.0:15000")
	require.Contains(t, out, "port: 8080")
	require.Contains(t, out, "listeners: []")
}

func TestStandaloneChartConfigYamlPrecedence(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `config:
  config:
    adminAddr: ignored
configYaml: |
  config:
    adminAddr: 0.0.0.0:16000
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "adminAddr: 0.0.0.0:16000")
	require.NotContains(t, out, "adminAddr: ignored")
}

func TestStandaloneChartPostgresDirectURL(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `database:
  type: postgres
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "url: postgres://agw:secret@postgres.default.svc:5432/agw")
	require.NotContains(t, out, "sqlite:///config/data.db")
}

func TestStandaloneChartGlobalImageRegistry(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `global:
  imageRegistry: registry.internal.example.com
image:
  registry: cr.agentgateway.dev
  repository: platform/agentgateway
  tag: 1.2.3
configBootstrap:
  image:
    registry: docker.io
    repository: library/busybox
    tag: 1.36
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, `image: "registry.internal.example.com/platform/agentgateway:v1.2.3"`)
	require.Contains(t, out, `image: "registry.internal.example.com/library/busybox:1.36"`)
	require.NotContains(t, out, "cr.agentgateway.dev/platform/agentgateway")
	require.NotContains(t, out, "docker.io/library/busybox")
}

func TestStandaloneChartPerImageRegistries(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `image:
  registry: registry.one.example.com
  repository: platform/agentgateway
  tag: dev
configBootstrap:
  image:
    registry: registry.two.example.com
    repository: library/busybox
    tag: 1.36
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, `image: "registry.one.example.com/platform/agentgateway:dev"`)
	require.Contains(t, out, `image: "registry.two.example.com/library/busybox:1.36"`)
}

func TestStandaloneChartConfigBootstrapOverwriteOverride(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `configBootstrap:
  overwrite: true
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, `if [ "true" = "true" ] || [ ! -f /config/config.yaml ]; then`)
}

func TestStandaloneChartRejectsSQLiteReplicas(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `replicaCount: 2
database:
  type: sqlite
`)
	require.Error(t, err)
	require.Contains(t, stderr, "sqlite database mode supports only replicaCount=1")
}

func TestStandaloneChartRejectsReplicaConfigYamlWithSQLiteDatabase(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `replicaCount: 2
persistence:
  accessModes:
  - ReadWriteMany
database:
  type: postgres
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
configYaml: |
  config:
    database:
      url: sqlite:///config/data.db
`)
	require.Error(t, err)
	require.Contains(t, stderr, "replicaCount > 1 requires config.database.url to be explicitly postgres")
}

func TestStandaloneChartRejectsReplicaStructuredConfigWithSQLiteDatabase(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `replicaCount: 2
persistence:
  accessModes:
  - ReadWriteMany
database:
  type: postgres
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
config:
  config:
    database:
      url: sqlite:///config/data.db
`)
	require.Error(t, err)
	require.Contains(t, stderr, "replicaCount > 1 requires config.database.url to be explicitly postgres")
}

func TestStandaloneChartRejectsReplicasWithoutPersistentRWXConfig(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `replicaCount: 2
persistence:
  enabled: false
database:
  type: postgres
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
`)
	require.Error(t, err)
	require.Contains(t, stderr, "replicaCount > 1 requires persistence.enabled=true and an RWX volume for shared /config")
}

func TestStandaloneChartRejectsReplicasWithoutReadWriteMany(t *testing.T) {
	_, stderr, err := renderStandaloneChart(t, `replicaCount: 2
database:
  type: postgres
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
`)
	require.Error(t, err)
	require.Contains(t, stderr, "replicaCount > 1 requires persistence.accessModes to include ReadWriteMany or persistence.existingClaim to reference an RWX volume")
}

func TestStandaloneChartAllowsReplicasWithPostgresAndReadWriteMany(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `replicaCount: 2
persistence:
  storageClassName: efs-sc
  accessModes:
  - ReadWriteMany
database:
  type: postgres
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "replicas: 2")
	require.Contains(t, out, "- ReadWriteMany")
	require.Contains(t, out, `storageClassName: "efs-sc"`)
	require.Contains(t, out, "url: postgres://agw:secret@postgres.default.svc:5432/agw")
}

func TestStandaloneChartAllowsReplicasWithPostgresAndExistingClaim(t *testing.T) {
	out, stderr, err := renderStandaloneChart(t, `replicaCount: 2
persistence:
  existingClaim: agw-rwx-config
database:
  type: postgres
  postgres:
    url: postgres://agw:secret@postgres.default.svc:5432/agw
`)
	require.NoError(t, err, "helm template failed: %s", stderr)
	require.Contains(t, out, "replicas: 2")
	require.Contains(t, out, "claimName: agw-rwx-config")
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
	require.Contains(t, out, "name: agentgateway-standalone-private-listener")
	require.Contains(t, out, "name: agentgateway-standalone-public-listener")
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
	out, stderr, err := renderStandaloneChart(t, `admin:
  service:
    type: LoadBalancer
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-type: nlb
    extraLabels:
      plane: admin
    clusterIPs:
    - 10.96.0.10
    externalIPs:
    - 203.0.113.10
    externalName: admin.example.com
    loadBalancerIP: 198.51.100.10
    loadBalancerSourceRanges:
    - 10.0.0.0/8
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
gateway:
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
	require.Contains(t, out, "plane: admin")
	require.Contains(t, out, "plane: gateway")
	require.Contains(t, out, "clusterIPs:\n    - 10.96.0.10")
	require.Contains(t, out, "clusterIPs:\n    - 10.96.0.20")
	require.Contains(t, out, "externalName: admin.example.com")
	require.Contains(t, out, "loadBalancerIP: 198.51.100.10")
	require.Contains(t, out, "loadBalancerIP: 198.51.100.20")
	require.Contains(t, out, "healthCheckNodePort: 32100")
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
