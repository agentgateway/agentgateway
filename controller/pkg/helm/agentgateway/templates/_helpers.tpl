
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
All labels including selector labels, standard labels, and custom gateway labels
*/}}
{{- define "kgateway.gateway.allLabels" -}}
{{- $gateway := .Values.agentgateway -}}
{{- $labels := merge (dict
  "app.kubernetes.io/managed-by" "agentgateway"
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

{{/*
Shared Gateway workload pod template
*/}}
{{- define "kgateway.gateway.podTemplate" -}}
{{- $gateway := .Values.agentgateway }}
{{- $promAnnotations := (dict
  "prometheus.io/path" "/metrics"
  "prometheus.io/port" "15020"
  "prometheus.io/scrape" "true")
}}
metadata:
  annotations:
  {{- /* Add managed resource checksums to trigger rollout when config changes */}}
  {{- $managedChecksums := dict "checksum/config" (include (print $.Template.BasePath "/configmap.yaml") . | sha256sum) }}
  {{- if and $gateway.xds.tls $gateway.xds.tls.enabled }}
  {{- $_ := set $managedChecksums "checksum/xds-ca" (include (print $.Template.BasePath "/xds-ca-configmap.yaml") . | sha256sum) }}
  {{- end }}
  {{- toYaml (merge
      $managedChecksums
      (deepCopy ($gateway.gatewayAnnotations | default dict))
      $promAnnotations
   ) | nindent 4 }}
  labels:
    {{- toYaml (merge
      (dict "gateway.networking.k8s.io/gateway-class-name" .Values.agentgateway.gatewayClassName)
      (include "kgateway.gateway.selectorLabels" . | fromYaml)
      ($gateway.gatewayLabels | default dict)
      ((hasKey $gateway "istio") | ternary (dict "istio.io/dataplane-mode" "none" "sidecar.istio.io/inject" "false") (dict))
    ) | nindent 4 }}
spec:
  serviceAccountName: {{ include "kgateway.gateway.fullname" . }}
  securityContext:
    sysctls:
      - name: net.ipv4.ip_unprivileged_port_start
        value: "0"
  containers:
    - name: agentgateway
      image: "{{ template "kgateway.gateway.image" $gateway.image }}"
      {{- if $gateway.image.pullPolicy }}
      imagePullPolicy: {{ $gateway.image.pullPolicy | quote }}
      {{- end }}
      securityContext:
        allowPrivilegeEscalation: false
        capabilities:
          drop:
            - ALL
        readOnlyRootFilesystem: true
        runAsNonRoot: true
        runAsUser: 10101
      readinessProbe:
        httpGet:
          path: /healthz/ready
          port: 15021
        periodSeconds: 10
      startupProbe:
        failureThreshold: 60
        httpGet:
          path: /healthz/ready
          port: 15021
        periodSeconds: 1
        successThreshold: 1
        timeoutSeconds: 2
      {{- /*  Note: this is not *all* ports, since those will be dynamically set. However, this is the static port which is useful for setting up monitoring */}}
      ports:
      - containerPort: 15020
        name: metrics
        protocol: TCP
      {{- with $gateway.resources }}
      resources:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      args:
        - -f
        - /config/config.yaml
      env:
      {{- /* Build a set of user-specified env var names for deduplication */}}
      {{- $userEnvNames := dict }}
      {{- range $gateway.env }}
        {{- $_ := set $userEnvNames .name true }}
      {{- end }}
      {{- /* Template-based defaults (skip if user overrides) */}}
        {{- if not (hasKey $userEnvNames "TERMINATION_GRACE_PERIOD_SECONDS") }}
        - name: TERMINATION_GRACE_PERIOD_SECONDS
          value: "{{($gateway.shutdown).max|default 60}}"
        {{- end }}
        {{- if not (hasKey $userEnvNames "CONNECTION_MIN_TERMINATION_DEADLINE") }}
        - name: CONNECTION_MIN_TERMINATION_DEADLINE
          value: "{{ ($gateway.shutdown).min | default 10 }}s"
        {{- end }}
        {{- if not (hasKey $userEnvNames "NODE_NAME") }}
        - name: NODE_NAME
          valueFrom:
            fieldRef:
              fieldPath: spec.nodeName
        {{- end }}
        {{- if not (hasKey $userEnvNames "POD_NAMESPACE") }}
        - name: POD_NAMESPACE
          valueFrom:
            fieldRef:
              fieldPath: metadata.namespace
        {{- end }}
        {{- if not (hasKey $userEnvNames "POD_NAME") }}
        - name: POD_NAME
          valueFrom:
            fieldRef:
              fieldPath: metadata.name
        {{- end }}
        {{- if not (hasKey $userEnvNames "RUST_BACKTRACE") }}
        - name: RUST_BACKTRACE
          value: "1"
        {{- end }}
        {{- if not (hasKey $userEnvNames "RUST_LOG") }}
        - name: RUST_LOG
          value: {{ ($gateway.logging).level | default "info" | quote }}
        {{- end }}
        {{- if $gateway.sessionKeySecretName }}
        - name: SESSION_KEY
          valueFrom:
            secretKeyRef:
              name: {{ required "agentgateway.sessionKeySecretName is required" $gateway.sessionKeySecretName }}
              key: key
        {{- end }}
        - name: XDS_ADDRESS
          {{- if and $gateway.xds.tls $gateway.xds.tls.enabled }}
          value: "https://{{ $gateway.xds.host }}:{{ $gateway.xds.port }}"
          {{- else }}
          value: "http://{{ $gateway.xds.host }}:{{ $gateway.xds.port }}"
          {{- end }}
        {{- if and $gateway.xds.tls $gateway.xds.tls.enabled }}
        - name: XDS_ROOT_CA
          value: "/etc/xds-tls/ca.crt"
        {{- end }}
        {{- if not (hasKey $userEnvNames "NAMESPACE") }}
        - name: NAMESPACE
          valueFrom:
            fieldRef:
              fieldPath: metadata.namespace
        {{- end }}
        {{- if not (hasKey $userEnvNames "GATEWAY") }}
        - name: GATEWAY
          value: {{ include "kgateway.gateway.fullname" . }}
        {{- end }}
        {{- if not (hasKey $userEnvNames "CPU_LIMIT") }}
        - name: CPU_LIMIT
          valueFrom:
            resourceFieldRef:
              resource: limits.cpu
              divisor: "1"
        {{- end }}
        {{- if not (hasKey $userEnvNames "INSTANCE_IP") }}
        - name: INSTANCE_IP
          valueFrom:
            fieldRef:
              fieldPath: status.podIP
        {{- end }}
        {{- if not (hasKey $userEnvNames "SERVICE_ACCOUNT") }}
        - name: SERVICE_ACCOUNT
          valueFrom:
            fieldRef:
              fieldPath: spec.serviceAccountName
        {{- end }}
        {{- if and (not (hasKey $userEnvNames "CA_ADDRESS")) (hasKey $gateway "istio") }}
        - name: CA_ADDRESS
          value: {{ $gateway.istio.caAddress | default "https://istiod.istio-system.svc:15012" | quote }}
        {{- end }}
        {{- if and (not (hasKey $userEnvNames "TRUST_DOMAIN")) (hasKey $gateway "istio") }}
        - name: TRUST_DOMAIN
          value: {{ $gateway.istio.trustDomain | default "cluster.local" | quote }}
        {{- end }}
        {{- if and (not (hasKey $userEnvNames "ADDITIONAL_TRUST_DOMAINS")) (hasKey $gateway "istio") $gateway.istio.additionalTrustDomains }}
        - name: ADDITIONAL_TRUST_DOMAINS
          value: {{ $gateway.istio.additionalTrustDomains | join "," | quote }}
        {{- end }}
        {{- if and (not (hasKey $userEnvNames "CLUSTER_ID")) (hasKey $gateway "istio") $gateway.istio.clusterId }}
        - name: CLUSTER_ID
          value: {{ $gateway.istio.clusterId | quote }}
        {{- end }}
        {{- if and (not (hasKey $userEnvNames "NETWORK")) (hasKey $gateway "istio") $gateway.istio.network }}
        - name: NETWORK
          value: {{ $gateway.istio.network | quote }}
        {{- end }}
      {{- $catalogPaths := list }}
      {{- range ($gateway.modelCatalog).sources }}
      {{- if .configMap }}
      {{- $catalogPaths = append $catalogPaths (printf "/etc/agentgateway/model-catalog/%s/%s.json" .configMap.name .configMap.name) }}
      {{- end }}
      {{- end }}
      {{- if $catalogPaths }}
        {{- if not (hasKey $userEnvNames "MODEL_CATALOG_PATHS") }}
        - name: MODEL_CATALOG_PATHS
          value: {{ $catalogPaths | join "," | quote }}
        {{- end }}
      {{- end }}
      {{- /* User-specified env vars */}}
      {{- with $gateway.env }}
        {{- toYaml . | nindent 8 }}
      {{- end }}
      volumeMounts:
        - name: config-volume
          mountPath: /config
        # Make /tmp writeable, needed for pprof
        - name: tmp
          mountPath: /tmp
        - name: xds-token
          mountPath: /var/run/secrets/xds-tokens
          readOnly: true
        {{- if and $gateway.xds.tls $gateway.xds.tls.enabled }}
        - name: xds-ca
          mountPath: /etc/xds-tls
          readOnly: true
        {{- end }}
        {{- if hasKey $gateway "istio" }}
        - mountPath: /var/run/secrets/istio
          name: istiod-ca-cert
          readOnly: true
        - mountPath: /var/run/secrets/tokens
          name: istio-token
          readOnly: true
        {{- end }}
        {{- range ($gateway.modelCatalog).sources }}
        {{- if .configMap }}
        - name: model-catalog-{{ .configMap.name }}
          mountPath: /etc/agentgateway/model-catalog/{{ .configMap.name }}
          readOnly: true
        {{- end }}
        {{- end }}
  volumes:
    - name: config-volume
      configMap:
        name: {{ include "kgateway.gateway.fullname" . }}
    - name: xds-token
      projected:
        sources:
        - serviceAccountToken:
            audience: agentgateway
            expirationSeconds: 43200
            path: xds-token
    - name: tmp
      emptyDir: {}
    {{- if and $gateway.xds.tls $gateway.xds.tls.enabled }}
    - name: xds-ca
      configMap:
        name: {{ include "kgateway.gateway.fullname" . }}-xds-ca
        items:
        - key: ca.crt
          path: ca.crt
    {{- end }}
    {{- if hasKey $gateway "istio" }}
    - configMap:
        name: istio-ca-root-cert
      name: istiod-ca-cert
    - name: istio-token
      projected:
        sources:
          - serviceAccountToken:
              audience: istio-ca
              expirationSeconds: 43200
              path: istio-token
    {{- end }}
    {{- range ($gateway.modelCatalog).sources }}
    {{- if .configMap }}
    - name: model-catalog-{{ .configMap.name }}
      configMap:
        name: {{ .configMap.name }}
        items:
        - key: {{ .configMap.key | default "catalog.json" }}
          path: {{ .configMap.name }}.json
    {{- end }}
    {{- end }}
  terminationGracePeriodSeconds: {{($gateway.shutdown).max|default 60|int}}
{{- end -}}
