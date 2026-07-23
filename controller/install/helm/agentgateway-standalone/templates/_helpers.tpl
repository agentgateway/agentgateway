{{- define "agentgateway-standalone.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "agentgateway-standalone.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
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

{{- define "agentgateway-standalone.imageRef" -}}
{{- $root := index . "root" -}}
{{- $image := index . "image" -}}
{{- $tag := index . "tag" | toString -}}
{{- $registry := $root.Values.global.imageRegistry | default $image.registry -}}
{{- printf "%s/%s:%s" $registry $image.repository $tag -}}
{{- end }}

{{- define "agentgateway-standalone.mainImage" -}}
{{- $tag := include "agentgateway-standalone.imageTag" (.Values.image.tag | default .Chart.AppVersion) -}}
{{- include "agentgateway-standalone.imageRef" (dict "root" . "image" .Values.image "tag" $tag) -}}
{{- end }}

{{- define "agentgateway-standalone.configBootstrapImage" -}}
{{- include "agentgateway-standalone.imageRef" (dict "root" . "image" .Values.configBootstrap.image "tag" .Values.configBootstrap.image.tag) -}}
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

{{- define "agentgateway-standalone.databaseUrl" -}}
{{- if eq .Values.database.type "sqlite" -}}
{{- printf "sqlite://%s" .Values.database.sqlite.path -}}
{{- else if eq .Values.database.type "postgres" -}}
{{- required "database.postgres.url is required when database.type=postgres" .Values.database.postgres.url -}}
{{- else -}}
{{- fail "database.type must be one of: sqlite, postgres" -}}
{{- end -}}
{{- end }}

{{- define "agentgateway-standalone.configYaml" -}}
{{- if .Values.configYaml -}}
{{ .Values.configYaml }}
{{- else if .Values.config -}}
{{ toYaml .Values.config }}
{{- else -}}
config:
  adminAddr: 0.0.0.0:{{ .Values.admin.service.port }}
  database:
    url: {{ include "agentgateway-standalone.databaseUrl" . }}
binds:
- port: 8080
  listeners: []
- port: 8443
  listeners: []
{{- end -}}
{{- end }}

{{- define "agentgateway-standalone.effectiveDatabaseUrl" -}}
{{- $renderedConfig := include "agentgateway-standalone.configYaml" . | fromYaml -}}
{{- if not (kindIs "map" $renderedConfig) -}}
{{- "" -}}
{{- else -}}
{{- $config := get $renderedConfig "config" -}}
{{- if not (kindIs "map" $config) -}}
{{- "" -}}
{{- else -}}
{{- $database := get $config "database" -}}
{{- if not (kindIs "map" $database) -}}
{{- "" -}}
{{- else -}}
{{- get $database "url" | default "" -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end }}

{{- define "agentgateway-standalone.validate" -}}
{{- if gt (int .Values.replicaCount) 1 -}}
{{- $databaseUrl := include "agentgateway-standalone.effectiveDatabaseUrl" . | trim -}}
{{- if regexMatch "^sqlite://" $databaseUrl -}}
{{- fail (printf "sqlite database mode supports only replicaCount=1; replicaCount > 1 requires config.database.url to be explicitly postgres:// or postgresql:// (got %s)" $databaseUrl) -}}
{{- end -}}
{{- if not (regexMatch "^postgres(ql)?://" $databaseUrl) -}}
{{- fail (printf "replicaCount > 1 requires config.database.url to be explicitly postgres:// or postgresql:// (got %q)" $databaseUrl) -}}
{{- end -}}
{{- if not .Values.persistence.enabled -}}
{{- fail "replicaCount > 1 requires persistence.enabled=true and an RWX volume for shared /config" -}}
{{- end -}}
{{- if and (not .Values.persistence.existingClaim) (not (has "ReadWriteMany" .Values.persistence.accessModes)) -}}
{{- fail "replicaCount > 1 requires persistence.accessModes to include ReadWriteMany or persistence.existingClaim to reference an RWX volume" -}}
{{- end -}}
{{- end -}}
{{- end }}
