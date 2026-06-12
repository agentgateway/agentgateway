package remotecache

import "github.com/agentgateway/agentgateway/controller/pkg/metrics"

const (
	subsystem  = "remotecache"
	codecLabel = "codec"
)

// hydrationParseErrors counts ConfigMaps that failed to parse on hydration;
// a persistently nonzero value points to corrupt or schema-mismatched
// ConfigMaps needing operator attention.
var hydrationParseErrors = metrics.NewCounter(
	metrics.CounterOpts{
		Subsystem: subsystem,
		Name:      "hydration_parse_errors_total",
		Help:      "Total ConfigMaps that failed to parse during remote-cache hydration, labeled by codec (jwks, oidc, ...).",
	},
	[]string{codecLabel},
)
