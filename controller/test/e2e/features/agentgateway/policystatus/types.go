//go:build e2e

package policystatus

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

var (
	// manifests
	policyWithGwManifest        = filepath.Join(fsutils.MustGetThisDir(), "testdata", "policy-with-gw.yaml")
	policyWithMissingGwManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "policy-with-missing-gw.yaml")

	setup = base.TestCase{
		Manifests: []string{policyWithGwManifest},
	}

	testCases = map[string]*base.TestCase{
		"TestAgwPolicyClearStaleStatus": {},
	}
)
