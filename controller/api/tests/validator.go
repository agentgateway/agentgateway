package tests

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"istio.io/istio/pkg/config/crd"
	"istio.io/istio/pkg/test/util/assert"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

func NewKgatewayValidator(t *testing.T) *crd.Validator {
	root := fsutils.GetModuleRoot()
	dirs := []string{}
	agentgatewayDir, err := os.ReadDir(filepath.Join(root, "install/helm/agentgateway-crds/templates/"))
	assert.NoError(t, err)
	for _, d := range agentgatewayDir {
		if strings.HasSuffix(d.Name(), ".yaml") {
			dirs = append(dirs, filepath.Join(root, "install/helm/agentgateway-crds/templates", d.Name()))
		}
	}
	v, err := crd.NewValidatorFromFiles(
		dirs...,
	)
	if err != nil {
		t.Fatal(err)
	}
	return v
}
