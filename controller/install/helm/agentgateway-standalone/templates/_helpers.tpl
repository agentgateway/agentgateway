{{- define "agentgateway-standalone.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "agentgateway-standalone.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "agentgateway-standalone.namespace" -}}
{{- .Values.namespaceOverride | default .Release.Namespace }}
{{- end }}

{{- define "agentgateway-standalone.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "agentgateway-standalone.selectorLabels" -}}
app.kubernetes.io/name: {{ include "agentgateway-standalone.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: standalone
{{- end }}

{{- define "agentgateway-standalone.labels" -}}
helm.sh/chart: {{ include "agentgateway-standalone.chart" . }}
{{ include "agentgateway-standalone.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- with .Values.commonLabels }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{- define "agentgateway-standalone.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "agentgateway-standalone.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "agentgateway-standalone.imageTag" -}}
{{- $tag := . -}}
{{- if hasPrefix "v" $tag -}}
{{- $tag -}}
{{- else if regexMatch "^[0-9]+\\.[0-9]+\\..*$" $tag -}}
{{- printf "v%s" $tag -}}
{{- else -}}
{{- $tag -}}
{{- end -}}
{{- end }}

{{- define "agentgateway-standalone.mainImage" -}}
{{- if kindIs "string" .Values.image -}}
{{- required "image must not be empty" .Values.image -}}
{{- else if kindIs "map" .Values.image -}}
{{- $tag := include "agentgateway-standalone.imageTag" (.Values.image.tag | default .Chart.AppVersion) -}}
{{- printf "%s/%s:%s" .Values.image.registry .Values.image.repository $tag -}}
{{- else -}}
{{- fail "image must be a string or mapping" -}}
{{- end -}}
{{- end }}

{{- define "agentgateway-standalone.mainImagePullPolicy" -}}
{{- if kindIs "string" .Values.image -}}
IfNotPresent
{{- else if kindIs "map" .Values.image -}}
{{- .Values.image.pullPolicy | default "IfNotPresent" -}}
{{- else -}}
{{- fail "image must be a string or mapping" -}}
{{- end -}}
{{- end }}

{{- define "agentgateway-standalone.serviceSpecFields" -}}
{{- with .clusterIP }}
clusterIP: {{ . }}
{{- end }}
{{- with .clusterIPs }}
clusterIPs:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .externalIPs }}
externalIPs:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .externalName }}
externalName: {{ . }}
{{- end }}
{{- with .loadBalancerIP }}
loadBalancerIP: {{ . }}
{{- end }}
{{- with .loadBalancerSourceRanges }}
loadBalancerSourceRanges:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .loadBalancerClass }}
loadBalancerClass: {{ . }}
{{- end }}
{{- with .externalTrafficPolicy }}
externalTrafficPolicy: {{ . }}
{{- end }}
{{- with .internalTrafficPolicy }}
internalTrafficPolicy: {{ . }}
{{- end }}
{{- if not (kindIs "invalid" .healthCheckNodePort) }}
healthCheckNodePort: {{ .healthCheckNodePort }}
{{- end }}
{{- with .sessionAffinity }}
sessionAffinity: {{ . }}
{{- end }}
{{- with .sessionAffinityConfig }}
sessionAffinityConfig:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .ipFamilies }}
ipFamilies:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .ipFamilyPolicy }}
ipFamilyPolicy: {{ . }}
{{- end }}
{{- if .publishNotReadyAddresses }}
publishNotReadyAddresses: true
{{- end }}
{{- if not (kindIs "invalid" .allocateLoadBalancerNodePorts) }}
allocateLoadBalancerNodePorts: {{ .allocateLoadBalancerNodePorts }}
{{- end }}
{{- with .trafficDistribution }}
trafficDistribution: {{ . }}
{{- end }}
{{- end }}

{{- define "agentgateway-standalone.baseConfig" -}}
{{- if .Values.config -}}
{{ toYaml .Values.config }}
{{- else -}}
gateways:
  default:
    port: 4000
ui: {}
llm:
  models: []
mcp:
  targets: []
{{- end -}}
{{- end }}

{{- define "agentgateway-standalone.renderedConfig" -}}
{{- $renderedConfig := include "agentgateway-standalone.baseConfig" . | fromYaml -}}
{{- if not (kindIs "map" $renderedConfig) -}}
{{- fail "config must render to a YAML mapping" -}}
{{- end -}}
{{- $config := get $renderedConfig "config" | default dict -}}
{{- if not (kindIs "map" $config) -}}
{{- fail "config.config must be a YAML mapping" -}}
{{- end -}}
{{- if eq .Values.mode "database" -}}
{{- $_ := set $config "database" (dict "url" .Values.database.postgres.url) -}}
{{- $_ := set $config "storage" (dict "mode" "hybrid") -}}
{{- else -}}
{{- $_ := set $config "storage" (dict "mode" "file") -}}
{{- end }}
{{- $_ := set $renderedConfig "config" $config -}}
{{ toYaml $renderedConfig }}
{{- end }}

{{- define "agentgateway-standalone.validate" -}}
{{- $mode := .Values.mode -}}
{{- if not (has $mode (list "readonly" "database")) -}}
{{- fail (printf "mode must be one of: readonly, database (got %q)" $mode) -}}
{{- end -}}
{{- $postgresUrl := .Values.database.postgres.url | default "" -}}
{{- if eq $mode "database" -}}
{{- if not (regexMatch "^postgres(ql)?://" $postgresUrl) -}}
{{- fail (printf "mode=database requires database.postgres.url to start with postgres:// or postgresql:// (got %q)" $postgresUrl) -}}
{{- end -}}
{{- else if $postgresUrl -}}
{{- fail (printf "database.postgres.url is only supported when mode=database (got mode %q)" $mode) -}}
{{- end -}}
{{- end -}}
