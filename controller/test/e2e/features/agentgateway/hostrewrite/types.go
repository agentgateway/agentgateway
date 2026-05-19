//go:build e2e

package hostrewrite

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

const (
	backendHost     = "backend.agentgateway-base.svc.cluster.local"
	svcDefaultHost  = "svc-default.hostrewrite.test"
	svcNoneHost     = "svc-none.hostrewrite.test"
	svcAutoHost     = "svc-auto.hostrewrite.test"
	agbeDefaultHost = "agbe-default.hostrewrite.test"
	agbeNoneHost    = "agbe-none.hostrewrite.test"
	agbeAutoHost    = "agbe-auto.hostrewrite.test"
)

var (
	serviceRoutesManifest = getTestFile("service-routes.yaml")
	agbeRoutesManifest    = getTestFile("agbe-routes.yaml")

	setup = base.TestCase{}

	testCases = map[string]*base.TestCase{
		"TestServiceBackendHostRewrite": {
			Manifests: []string{serviceRoutesManifest},
		},
		"TestAgentgatewayBackendHostRewrite": {
			Manifests: []string{agbeRoutesManifest},
		},
	}
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
