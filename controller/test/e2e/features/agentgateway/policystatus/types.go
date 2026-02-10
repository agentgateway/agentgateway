//go:build e2e

package policystatus

import (
	"path/filepath"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
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
