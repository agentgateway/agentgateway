//go:build e2e

package tests

import (
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/a2a"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/aibackend"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/apikeyauth"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/backendtls"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/basicauth"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/configmap"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/csrf"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/extauth"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/extproc"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/jwtauth"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/mcp"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/policystatus"
	global_rate_limit "github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/rate_limit/global"
	local_rate_limit "github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/rate_limit/local"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/rbac"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/remotejwtauth"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/tracing"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/agentgateway/transformation"
)

func AgentgatewaySuiteRunner() e2e.SuiteRunner {
	agentgatewaySuiteRunner := e2e.NewSuiteRunner(false)

	agentgatewaySuiteRunner.Register("A2A", a2a.NewTestingSuite)
	agentgatewaySuiteRunner.Register("BasicRouting", agentgateway.NewTestingSuite)
	agentgatewaySuiteRunner.Register("BackendTLSPolicy", backendtls.NewTestingSuite)
	agentgatewaySuiteRunner.Register("BasicAuth", basicauth.NewTestingSuite)
	agentgatewaySuiteRunner.Register("ApiKeyAuth", apikeyauth.NewTestingSuite)
	agentgatewaySuiteRunner.Register("JwtAuth", jwtauth.NewTestingSuite)
	agentgatewaySuiteRunner.Register("CSRF", csrf.NewTestingSuite)
	agentgatewaySuiteRunner.Register("Extauth", extauth.NewTestingSuite)
	agentgatewaySuiteRunner.Register("Extproc", extproc.NewTestingSuite)
	agentgatewaySuiteRunner.Register("LocalRateLimit", local_rate_limit.NewTestingSuite)
	agentgatewaySuiteRunner.Register("GlobalRateLimit", global_rate_limit.NewTestingSuite)
	agentgatewaySuiteRunner.Register("RBAC", rbac.NewTestingSuite)
	agentgatewaySuiteRunner.Register("MCP", mcp.NewTestingSuite)
	agentgatewaySuiteRunner.Register("AIBackend", aibackend.NewTestingSuite)
	agentgatewaySuiteRunner.Register("Transformation", transformation.NewTestingSuite)
	agentgatewaySuiteRunner.Register("RemoteJwtAuth", remotejwtauth.NewTestingSuite)
	agentgatewaySuiteRunner.Register("ConfigMap", configmap.NewTestingSuite) // redeploys by need
	agentgatewaySuiteRunner.Register("Tracing", tracing.NewTestingSuite)
	agentgatewaySuiteRunner.Register("PolicyStatus", policystatus.NewTestingSuite)

	return agentgatewaySuiteRunner
}
