package deployer

import (
	"testing"

	"github.com/stretchr/testify/require"
	"helm.sh/helm/v4/pkg/engine"
	corev1 "k8s.io/api/core/v1"
	"sigs.k8s.io/yaml"
)

func TestSpiffeConfigMapTrustDomainOverride(t *testing.T) {
	tests := []struct {
		name  string
		typed *bool
		raw   *bool
		want  *bool
	}{
		{name: "unset"},
		{name: "explicit false", typed: new(false), want: new(false)},
		{name: "explicit true", typed: new(true), want: new(true)},
		{name: "raw true", raw: new(true), want: new(true)},
		{name: "raw false", raw: new(false), want: new(false)},
		{name: "false overrides raw true", typed: new(false), raw: new(true), want: new(false)},
		{name: "true overrides raw false", typed: new(true), raw: new(false), want: new(true)},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			chart, err := LoadAgentgatewayChart()
			require.NoError(t, err)
			templates := chart.Templates[:0]
			for _, template := range chart.Templates {
				if template.Name == "templates/configmap.yaml" || template.Name == "templates/_helpers.tpl" {
					templates = append(templates, template)
				}
			}
			chart.Templates = templates
			spiffe := map[string]any{}
			if tt.typed != nil {
				spiffe["allowAdditionalTrustDomains"] = *tt.typed
			}
			rawSpiffe := map[string]any{"endpoint": "unix:///raw.sock"}
			if tt.raw != nil {
				rawSpiffe["allowAdditionalTrustDomains"] = *tt.raw
			}
			rendered, err := engine.Render(chart, map[string]any{
				"Values": map[string]any{
					"agentgateway": map[string]any{
						"name":             "test",
						"gatewayClassName": "agentgateway",
						"spiffe":           spiffe,
						"rawConfig": map[string]any{
							"config": map[string]any{
								"spiffe": rawSpiffe,
								"tracing": map[string]any{
									"otlpEndpoint": "http://jaeger:4317",
								},
							},
						},
					},
				},
				"Chart": chart.Metadata,
			})
			require.NoError(t, err)
			var configMap corev1.ConfigMap
			require.NoError(t, yaml.Unmarshal([]byte(rendered["agentgateway/templates/configmap.yaml"]), &configMap))
			var config struct {
				Config struct {
					Spiffe struct {
						Endpoint                    string `json:"endpoint"`
						AllowAdditionalTrustDomains *bool  `json:"allowAdditionalTrustDomains"`
					} `json:"spiffe"`
					Tracing struct {
						OTLPEndpoint string `json:"otlpEndpoint"`
					} `json:"tracing"`
				} `json:"config"`
			}
			require.NoError(t, yaml.Unmarshal([]byte(configMap.Data["config.yaml"]), &config))
			require.Equal(t, tt.want, config.Config.Spiffe.AllowAdditionalTrustDomains)
			require.Equal(t, "unix:///spiffe-workload-api/spire-agent.sock", config.Config.Spiffe.Endpoint)
			require.Equal(t, "http://jaeger:4317", config.Config.Tracing.OTLPEndpoint)
		})
	}
}
