//go:build e2e

package delegation

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

var (
	setupManifest                  = getTestFile("setup.yaml")
	basicDelegationManifest        = getTestFile("basic-delegation.yaml")
	delegationHeadersQueryManifest = getTestFile("delegation-headers-query.yaml")
	cyclicDelegationManifest       = getTestFile("cyclic-delegation.yaml")
	recursiveDelegationManifest    = getTestFile("recursive-delegation.yaml")
	unresolvedChildManifest        = getTestFile("unresolved-child.yaml")
	multipleParentsManifest        = getTestFile("multiple-parents.yaml")

	setup = base.TestCase{
		Manifests: []string{setupManifest},
	}

	testCases = map[string]*base.TestCase{
		"TestBasicDelegation": {
			Manifests: []string{basicDelegationManifest},
		},
		"TestDelegationWithHeadersAndQueryParams": {
			Manifests: []string{delegationHeadersQueryManifest},
		},
		"TestCyclicDelegation": {
			Manifests: []string{cyclicDelegationManifest},
		},
		"TestRecursiveDelegation": {
			Manifests: []string{recursiveDelegationManifest},
		},
		"TestUnresolvedChild": {
			Manifests: []string{unresolvedChildManifest},
		},
		"TestMultipleParents": {
			Manifests: []string{multipleParentsManifest},
		},
	}
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
