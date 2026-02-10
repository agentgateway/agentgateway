{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "kgateway.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Data-plane related macros:
*/}}


{{/*
Generate a unique name for the gateway that is RFC1123 label compliant (<64 chars)
*/}}
{{- define "kgateway.gateway.safeLabelValue" -}}
{{- $name := . -}}
{{- if gt (len $name) 63 -}}
{{- $hash := $name | sha256sum | trunc 12 -}}
{{- printf "%s-%s" ($name | trunc 50 | trimSuffix "-") $hash -}}
{{- else -}}
{{- $name -}}
{{- end -}}
{{- end -}}

{{/*
Expand the name of the chart.
*/}}
{{- define "kgateway.gateway.name" -}}
{{ include "kgateway.gateway.safeLabelValue" (default .Values.gateway.nameOverride .Values.gateway.name) }}
{{- end }}

{{/*
Create a default fully qualified app name.
Use safeLabelValue because some Kubernetes name fields are limited to 63 chars (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "kgateway.gateway.fullname" -}}
{{ include "kgateway.gateway.safeLabelValue" (default .Values.gateway.nameOverride .Values.gateway.fullnameOverride) }}
{{- end }}

{{/*
Constant labels - labels that are stable across releases
*/}}
{{- define "kgateway.gateway.constLabels" -}}
kgateway: kube-gateway
{{- end }}


{{/*
Common labels
*/}}
{{- define "kgateway.gateway.labels" -}}
{{ include "kgateway.gateway.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
gateway.networking.k8s.io/gateway-class-name: {{ .Values.gateway.gatewayClassName }}
app.kubernetes.io/managed-by: kgateway
{{- end }}

{{- define "kgateway.gateway.podLabels" -}}
{{ include "kgateway.gateway.selectorLabels" . }}
gateway.networking.k8s.io/gateway-class-name: {{ .Values.gateway.gatewayClassName }}
{{- end }}


{{/*
Selector labels
*/}}
{{- define "kgateway.gateway.selectorLabels" -}}
app.kubernetes.io/name: {{ include "kgateway.gateway.name" . }}
app.kubernetes.io/instance: {{ include "kgateway.gateway.name" . }}
gateway.networking.k8s.io/gateway-name: {{ include "kgateway.gateway.name" . }}
{{- end }}

{{/*
Gateway name annotation - always contains the full gateway name
*/}}
{{- define "kgateway.gateway.gatewayNameAnnotation" -}}
gateway.kgateway.dev/gateway-full-name: {{ .Values.gateway.gatewayName }}
{{- end }}

{{/*
All labels including selector labels, standard labels, and custom gateway labels
*/}}
{{- define "kgateway.gateway.allLabels" -}}
{{- $gateway := .Values.gateway -}}
{{- $labels := merge (dict
  "kgateway" "kube-gateway"
  "app.kubernetes.io/managed-by" "kgateway"
  "gateway.networking.k8s.io/gateway-class-name" .Values.gateway.gatewayClassName
  )
  (include "kgateway.gateway.selectorLabels" . | fromYaml)
  ($gateway.gatewayLabels | default dict)
-}}
{{- if .Chart.AppVersion -}}
{{- $_ := set $labels "app.kubernetes.io/version" .Chart.AppVersion -}}
{{- end -}}
{{- $labels | toYaml -}}
{{- end -}}

{{/*
Return a container image value as a string
*/}}
{{- define "kgateway.gateway.image" -}}
{{- if not .repository -}}
{{- fail "an Image's repository must be present" -}}
{{- end -}}
{{- $image := "" -}}
{{- if .registry -}}
{{- $image = printf "%s/%s" .registry .repository -}}
{{- else -}}
{{- $image = printf "%s" .repository -}}
{{- end -}}
{{- if .tag -}}
{{- $image = printf "%s:%s" $image .tag -}}
{{- end -}}
{{- if .digest -}}
{{- $image = printf "%s@%s" $image .digest -}}
{{- end -}}
{{ $image }}
{{- end -}}
