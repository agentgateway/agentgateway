package tests

import (
	"fmt"
	"strings"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/tmpl"
)

func TestAttachments(t *testing.T) {
	type Attachments struct {
		Gateway    bool
		Port       bool
		Listener   bool
		Route      bool
		RouteRule  bool
		Backend    bool
		SubBackend bool
	}
	cases := []struct {
		name        string
		policy      string
		attachments Attachments
	}{
		{
			name: "frontend",
			policy: `frontend:
  tcp:
    keepalive: {}`,
			attachments: Attachments{
				Gateway:    true,
				Port:       false,
				Listener:   false,
				Route:      false,
				RouteRule:  false,
				Backend:    false,
				SubBackend: false,
			},
		},
		{
			name: "traffic",
			policy: `traffic:
  extProc:
    backendRef:
      name: ext-processor
      port: 80`,
			attachments: Attachments{
				Gateway:    true,
				Port:       false,
				Listener:   true,
				Route:      true,
				RouteRule:  true,
				Backend:    false,
				SubBackend: false,
			},
		},
		{
			name: "backend",
			policy: `backend:
  tls: {}`,
			attachments: Attachments{
				Gateway:    true,
				Port:       false,
				Listener:   true,
				Route:      true,
				RouteRule:  true,
				Backend:    true,
				SubBackend: true,
			},
		},
	}
	tm := `apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayPolicy
spec:
  {{if .ref}}targetRefs{{else}}targetSelectors{{end}}:
  - group: {{.group}}
    kind: {{.kind}}
    {{with .sectionName}}sectionName: {{.}}{{end}}
  {{if .ref}}
    name: t1
  {{else}}
    matchLabels:
      app: foo
  {{end}}
  {{.policy|nindent 2}}
`
	v := NewKgatewayValidator(t)
	for _, tt := range cases {
		for _, ref := range []bool{true, false} {
			sn := "ref"
			if !ref {
				sn = "selector"
			}
			t.Run(tt.name+"/"+sn, func(t *testing.T) {
				eval := func(ty string, want bool) {
					t.Run(strings.ReplaceAll(ty, "/", "-"), func(t *testing.T) {
						p := strings.Split(ty, "/")
						inp := map[string]any{
							"group":  p[0],
							"kind":   p[1],
							"policy": tt.policy,
							"ref":    ref,
						}
						if len(p) > 2 {
							inp["sectionName"] = p[2]
						}
						res := tmpl.EvaluateOrFail(t, tm, inp)
						vv := v.ValidateCustomResourceYAML(res, nil)
						assert.Equal(t, want, vv == nil, fmt.Sprintf("result: %v", vv))
					})
				}
				eval("gateway.networking.k8s.io/Gateway", tt.attachments.Gateway)
				//eval("gateway.networking.k8s.io/Gateway", tt.attachments.Port)
				eval("gateway.networking.k8s.io/Gateway/sec1", tt.attachments.Listener)
				eval("gateway.networking.k8s.io/HTTPRoute", tt.attachments.Route)
				eval("gateway.networking.k8s.io/HTTPRoute/sec1", tt.attachments.RouteRule)
				eval("agentgateway.dev/AgentgatewayBackend", tt.attachments.Backend)
				eval("agentgateway.dev/AgentgatewayBackend/sec1", tt.attachments.SubBackend)
			})
		}
	}
}
