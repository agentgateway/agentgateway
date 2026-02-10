//go:build e2e

package rbac

import (
	"net/http"
	"path/filepath"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const (
	namespace = "agentgateway-base"
)

var (
	// manifests
	rbacManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "cel-rbac.yaml")

	expectStatus200Success = &matchers.HttpResponse{
		StatusCode: http.StatusOK,
		Body:       nil,
	}
	expectRBACDenied = &matchers.HttpResponse{
		StatusCode: http.StatusForbidden,
		Body:       gomega.ContainSubstring("authorization failed"),
	}

	// Base test setup - common infrastructure for all tests
	setup = base.TestCase{
		Manifests: []string{rbacManifest},
	}

	// Individual test cases - test-specific manifests and resources
	testCases = map[string]*base.TestCase{}
)
