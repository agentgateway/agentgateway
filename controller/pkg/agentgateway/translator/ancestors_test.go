package translator_test

import (
	"bytes"
	"fmt"
	"strings"
	"testing"
	"text/template"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/test/util/assert"
	"k8s.io/apimachinery/pkg/types"

	apitests "github.com/agentgateway/agentgateway/controller/api/tests"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
)

const (
	ancestorDefaultNamespace = "default"
	ancestorGatewayGroup     = "gateway.networking.k8s.io"
	ancestorGatewayKind      = "Gateway"
	ancestorListenerSetKind  = "ListenerSet"
	ancestorBackendGroup     = "agentgateway.dev"
	ancestorBackendKind      = "AgentgatewayBackend"
	ancestorServiceKind      = "Service"
)

type ancestorRef struct {
	Group       string
	Kind        string
	Namespace   string
	Name        string
	Port        int
	SectionName string
}

type ancestorRoute struct {
	APIVersion  string
	Kind        string
	Name        string
	SectionName string
}

type ancestorTemplateData struct {
	Gateway     ancestorRef
	ListenerSet ancestorRef
	Parent      ancestorRef
	Target      ancestorRef
	Route       ancestorRoute
}

var ancestorTemplateFuncs = template.FuncMap{
	"isService": func(ref ancestorRef) bool {
		return ref.Kind == ancestorServiceKind
	},
}

var gatewayClassYAML = `apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: agentgateway
spec:
  controllerName: agentgateway.dev/agentgateway
`

var gatewayYAMLTemplate = template.Must(template.New("gateway").Parse(`apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: "{{ .Gateway.Name }}"
  namespace: "{{ .Gateway.Namespace }}"
spec:
  gatewayClassName: agentgateway
  listeners:
    - name: http
      protocol: HTTP
      port: 80
      allowedRoutes:
        namespaces:
          from: All
        kinds:
          - group: gateway.networking.k8s.io
            kind: HTTPRoute
          - group: gateway.networking.k8s.io
            kind: GRPCRoute
    - name: tcp
      protocol: TCP
      port: 90
      allowedRoutes:
        namespaces:
          from: All
        kinds:
          - group: gateway.networking.k8s.io
            kind: TCPRoute
    - name: tls
      protocol: TLS
      port: 443
      tls:
        mode: Passthrough
      allowedRoutes:
        namespaces:
          from: All
        kinds:
          - group: gateway.networking.k8s.io
            kind: TLSRoute
  allowedListeners:
    namespaces:
      from: All
`))

var listenerSetYAMLTemplate = template.Must(template.New("listenerset").Parse(`apiVersion: gateway.networking.k8s.io/v1
kind: ListenerSet
metadata:
  name: "{{ .ListenerSet.Name }}"
  namespace: "{{ .ListenerSet.Namespace }}"
spec:
  parentRef:
    group: "{{ .Gateway.Group }}"
    kind: "{{ .Gateway.Kind }}"
    namespace: "{{ .Gateway.Namespace }}"
    name: "{{ .Gateway.Name }}"
  listeners:
    - name: http
      protocol: HTTP
      port: 8080
      allowedRoutes:
        namespaces:
          from: All
        kinds:
          - group: gateway.networking.k8s.io
            kind: HTTPRoute
          - group: gateway.networking.k8s.io
            kind: GRPCRoute
    - name: tcp
      protocol: TCP
      port: 9090
      allowedRoutes:
        namespaces:
          from: All
        kinds:
          - group: gateway.networking.k8s.io
            kind: TCPRoute
    - name: tls
      protocol: TLS
      port: 9443
      tls:
        mode: Passthrough
      allowedRoutes:
        namespaces:
          from: All
        kinds:
          - group: gateway.networking.k8s.io
            kind: TLSRoute
`))

var httpRouteYAMLTemplate = template.Must(template.New("httproute").Funcs(ancestorTemplateFuncs).Parse(`apiVersion: gateway.networking.k8s.io/v1
kind: HTTPRoute
metadata:
  name: "{{ .Route.Name }}"
  namespace: "{{ .Parent.Namespace }}"
spec:
  parentRefs:
    - group: "{{ .Parent.Group }}"
      kind: "{{ .Parent.Kind }}"
      namespace: "{{ .Parent.Namespace }}"
      name: "{{ .Parent.Name }}"
      sectionName: "{{ .Route.SectionName }}"
  rules:
    - backendRefs:
        - {{- if not (isService .Target) }}
          group: "{{ .Target.Group }}"
          kind: "{{ .Target.Kind }}"
          {{- end }}
          namespace: "{{ .Target.Namespace }}"
          name: "{{ .Target.Name }}"
          {{- if isService .Target }}
          port: {{ .Target.Port }}
          {{- end }}
`))

var grpcRouteYAMLTemplate = template.Must(template.New("grpcroute").Funcs(ancestorTemplateFuncs).Parse(`apiVersion: gateway.networking.k8s.io/v1
kind: GRPCRoute
metadata:
  name: "{{ .Route.Name }}"
  namespace: "{{ .Parent.Namespace }}"
spec:
  parentRefs:
    - group: "{{ .Parent.Group }}"
      kind: "{{ .Parent.Kind }}"
      namespace: "{{ .Parent.Namespace }}"
      name: "{{ .Parent.Name }}"
      sectionName: "{{ .Route.SectionName }}"
  rules:
    - matches:
        - method:
            service: "example.Service"
            method: "Call"
      backendRefs:
        - {{- if not (isService .Target) }}
          group: "{{ .Target.Group }}"
          kind: "{{ .Target.Kind }}"
          {{- end }}
          namespace: "{{ .Target.Namespace }}"
          name: "{{ .Target.Name }}"
          {{- if isService .Target }}
          port: {{ .Target.Port }}
          {{- end }}
`))

var tcpRouteYAMLTemplate = template.Must(template.New("tcproute").Funcs(ancestorTemplateFuncs).Parse(`apiVersion: gateway.networking.k8s.io/v1alpha2
kind: TCPRoute
metadata:
  name: "{{ .Route.Name }}"
  namespace: "{{ .Parent.Namespace }}"
spec:
  parentRefs:
    - group: "{{ .Parent.Group }}"
      kind: "{{ .Parent.Kind }}"
      namespace: "{{ .Parent.Namespace }}"
      name: "{{ .Parent.Name }}"
      sectionName: "{{ .Route.SectionName }}"
  rules:
    - backendRefs:
        - {{- if not (isService .Target) }}
          group: "{{ .Target.Group }}"
          kind: "{{ .Target.Kind }}"
          {{- end }}
          namespace: "{{ .Target.Namespace }}"
          name: "{{ .Target.Name }}"
          {{- if isService .Target }}
          port: {{ .Target.Port }}
          {{- end }}
`))

var tlsRouteYAMLTemplate = template.Must(template.New("tlsroute").Funcs(ancestorTemplateFuncs).Parse(`apiVersion: gateway.networking.k8s.io/v1
kind: TLSRoute
metadata:
  name: "{{ .Route.Name }}"
  namespace: "{{ .Parent.Namespace }}"
spec:
  hostnames:
    - "example.com"
  parentRefs:
    - group: "{{ .Parent.Group }}"
      kind: "{{ .Parent.Kind }}"
      namespace: "{{ .Parent.Namespace }}"
      name: "{{ .Parent.Name }}"
      sectionName: "{{ .Route.SectionName }}"
  rules:
    - backendRefs:
        - {{- if not (isService .Target) }}
          group: "{{ .Target.Group }}"
          kind: "{{ .Target.Kind }}"
          {{- end }}
          namespace: "{{ .Target.Namespace }}"
          name: "{{ .Target.Name }}"
          {{- if isService .Target }}
          port: {{ .Target.Port }}
          {{- end }}
`))

var agentgatewayBackendYAMLTemplate = template.Must(template.New("agentgatewaybackend").Parse(`apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: "{{ .Target.Name }}"
  namespace: "{{ .Target.Namespace }}"
spec:
  static:
    host: example.com
    port: {{ .Target.Port }}
`))

var serviceYAMLTemplate = template.Must(template.New("service").Parse(`apiVersion: v1
kind: Service
metadata:
  name: "{{ .Target.Name }}"
  namespace: "{{ .Target.Namespace }}"
spec:
  ports:
    - name: http
      port: {{ .Target.Port }}
      targetPort: {{ .Target.Port }}
`))

func renderAncestorTemplate(t *testing.T, tmpl *template.Template, data ancestorTemplateData) string {
	t.Helper()

	var out bytes.Buffer
	require.NoError(t, tmpl.Execute(&out, data))
	return strings.TrimSpace(out.String())
}

func renderRouteYAML(t *testing.T, data ancestorTemplateData) string {
	t.Helper()

	switch data.Route.Kind {
	case "HTTPRoute":
		return renderAncestorTemplate(t, httpRouteYAMLTemplate, data)
	case "GRPCRoute":
		return renderAncestorTemplate(t, grpcRouteYAMLTemplate, data)
	case "TCPRoute":
		return renderAncestorTemplate(t, tcpRouteYAMLTemplate, data)
	case "TLSRoute":
		return renderAncestorTemplate(t, tlsRouteYAMLTemplate, data)
	default:
		t.Fatalf("unsupported route kind %q", data.Route.Kind)
		return ""
	}
}

func normalizeParentRef(parent *ancestorRef) {
	if parent.Namespace == "" {
		parent.Namespace = ancestorDefaultNamespace
	}
	if parent.Group == "" {
		parent.Group = ancestorGatewayGroup
	}
}

func normalizeTargetRef(target *ancestorRef) {
	if target.Namespace == "" {
		target.Namespace = ancestorDefaultNamespace
	}
	if target.Kind == ancestorServiceKind && target.Port == 0 {
		target.Port = 8080
	}
	if target.Kind == ancestorBackendKind {
		if target.Group == "" {
			target.Group = ancestorBackendGroup
		}
		if target.Port == 0 {
			target.Port = 8080
		}
	}
}

func gatewayRefForParent(parent ancestorRef) ancestorRef {
	if parent.Kind == ancestorGatewayKind {
		return parent
	}
	return ancestorRef{
		Group:     ancestorGatewayGroup,
		Kind:      ancestorGatewayKind,
		Namespace: parent.Namespace,
		Name:      "gateway",
	}
}

func routeForKind(kind string) ancestorRoute {
	switch kind {
	case "HTTPRoute":
		return ancestorRoute{
			APIVersion:  "gateway.networking.k8s.io/v1",
			Kind:        "HTTPRoute",
			Name:        "http-route",
			SectionName: "http",
		}
	case "GRPCRoute":
		return ancestorRoute{
			APIVersion:  "gateway.networking.k8s.io/v1",
			Kind:        "GRPCRoute",
			Name:        "grpc-route",
			SectionName: "http",
		}
	case "TCPRoute":
		return ancestorRoute{
			APIVersion:  "gateway.networking.k8s.io/v1alpha2",
			Kind:        "TCPRoute",
			Name:        "tcp-route",
			SectionName: "tcp",
		}
	case "TLSRoute":
		return ancestorRoute{
			APIVersion:  "gateway.networking.k8s.io/v1",
			Kind:        "TLSRoute",
			Name:        "tls-route",
			SectionName: "tls",
		}
	default:
		panic("unsupported route kind: " + kind)
	}
}

func ancestorTemplateDataFor(parent, target *ancestorRef, routeKind string) ancestorTemplateData {
	normalizeParentRef(parent)
	normalizeTargetRef(target)
	return ancestorTemplateData{
		Gateway: gatewayRefForParent(*parent),
		ListenerSet: ancestorRef{
			Group:     ancestorGatewayGroup,
			Kind:      ancestorListenerSetKind,
			Namespace: parent.Namespace,
			Name:      "listenerset",
		},
		Parent: *parent,
		Target: *target,
		Route:  routeForKind(routeKind),
	}
}

func renderAncestorYAML(t *testing.T, parent, target *ancestorRef, routeKind string) string {
	t.Helper()

	data := ancestorTemplateDataFor(parent, target, routeKind)
	docs := []string{
		gatewayClassYAML,
		renderAncestorTemplate(t, gatewayYAMLTemplate, data),
	}
	if data.Parent.Kind == ancestorListenerSetKind {
		docs = append(docs, renderAncestorTemplate(t, listenerSetYAMLTemplate, data))
	}
	docs = append(docs, renderRouteYAML(t, data))
	switch data.Target.Kind {
	case ancestorBackendKind:
		docs = append(docs, renderAncestorTemplate(t, agentgatewayBackendYAMLTemplate, data))
	case ancestorServiceKind:
		docs = append(docs, renderAncestorTemplate(t, serviceYAMLTemplate, data))
	default:
		t.Fatalf("unsupported target kind %q", data.Target.Kind)
	}
	return strings.Join(docs, "\n---\n")
}

func TestAncestors(t *testing.T) {
	validator := apitests.NewAgentgatewayValidator(t)
	validator.SkipMissing = true
	parents := []ancestorRef{{
		Kind: ancestorListenerSetKind,
		Name: "listenerset",
	}, {
		Kind: ancestorGatewayKind,
		Name: "gateway",
	}}
	routes := []string{"HTTPRoute", "GRPCRoute", "TCPRoute", "TLSRoute"}
	targets := []ancestorRef{
		{
			Kind: ancestorServiceKind,
			Name: "service",
			Port: 8080,
		},
		{
			Kind: ancestorBackendKind,
			Name: "backend",
		},
	}

	for _, p := range parents {
		for _, r := range routes {
			for _, tgt := range targets {
				t.Run(fmt.Sprintf("%v-%v-%v", p.Kind, r, tgt.Kind), func(t *testing.T) {
					yml := renderAncestorYAML(t, &p, &tgt, r)
					assert.NoError(t, validator.ValidateCustomResourceYAML(yml, nil))

					ctx := testutils.BuildMockPolicyContext(t, []any{yml})
					_, ri := testutils.Syncer(t, ctx)

					ancestors := ri.Outputs.References.Ancestors.List()
					assert.Equal(t, len(ancestors), 1)
					assert.Equal(t, len(ancestors[0].Objects), 1)
					got := ancestors[0].Objects[0]

					// No matter what the path is, we should have a route from Gateway <--> Backend
					assert.Equal(t, got.Gateway.Name, "gateway")
					assert.Equal(t, got.Backend.NamespacedName, tgt.NamespacedName())
					assert.Equal(t, got.Backend.Kind, tgt.Kind)
				})
			}
		}
	}
}

func (r ancestorRef) NamespacedName() types.NamespacedName {
	return types.NamespacedName{
		Namespace: r.Namespace,
		Name:      r.Name,
	}
}

func (r ancestorRoute) NamespacedName(namespace string) types.NamespacedName {
	return types.NamespacedName{
		Namespace: namespace,
		Name:      r.Name,
	}
}
