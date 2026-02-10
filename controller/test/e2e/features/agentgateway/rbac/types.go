//go:build e2e

package rbac

import (
	"net/http"
	"path/filepath"

	"github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
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
