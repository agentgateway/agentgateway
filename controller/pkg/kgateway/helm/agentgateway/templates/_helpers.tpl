
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
Create a default fully qualified app name.
Use safeLabelValue because some Kubernetes name fields are limited to 63 chars (by the DNS naming spec).
*/}}
{{- define "kgateway.gateway.name" -}}
{{- include "kgateway.gateway.safeLabelValue" (default .Values.agentgateway.name) }}
{{- end }}

{{/*
Create a default fully qualified app name.
Use safeLabelValue because some Kubernetes name fields are limited to 63 chars (by the DNS naming spec).
*/}}
{{- define "kgateway.gateway.fullname" -}}
{{- include "kgateway.gateway.safeLabelValue" (default .Values.agentgateway.name) }}

{{- end }}

{{/*
Selector labels
*/}}
{{- define "kgateway.gateway.selectorLabels" -}}
app.kubernetes.io/name: {{ include "kgateway.gateway.name" . }}
app.kubernetes.io/instance: {{ include "kgateway.gateway.fullname" . }}
gateway.networking.k8s.io/gateway-name: {{ include "kgateway.gateway.fullname" . }}
{{- end }}

{{/*
Gateway name annotation - always contains the full gateway name
*/}}
{{- define "kgateway.gateway.gatewayNameAnnotation" -}}
gateway.kgateway.dev/gateway-full-name: {{ .Values.agentgateway.name }}
{{- end }}

{{/*
All labels including selector labels, standard labels, and custom gateway labels
*/}}
{{- define "kgateway.gateway.allLabels" -}}
{{- $gateway := .Values.agentgateway -}}
{{- $labels := merge (dict
  "kgateway" "kube-gateway"
  "app.kubernetes.io/managed-by" "kgateway"
  "gateway.networking.k8s.io/gateway-class-name" .Values.agentgateway.gatewayClassName
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
