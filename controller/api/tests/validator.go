package tests

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"istio.io/istio/pkg/config/crd"
	"istio.io/istio/pkg/lazy"
	"istio.io/istio/pkg/test/util/assert"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
)

var validator = lazy.New(newAgentgatewayValidator)

func NewAgentgatewayValidator(t *testing.T) *crd.Validator {
	v, err := validator.Get()
	assert.NoError(t, err)
	return v
}

func newAgentgatewayValidator() (*crd.Validator, error) {
	root := fsutils.GetModuleRoot()
	dirs := []string{}
	agentgatewayDir, err := os.ReadDir(filepath.Join(root, "controller/install/helm/agentgateway-crds/templates/"))
	if err != nil {
		return nil, err
	}
	for _, d := range agentgatewayDir {
		if strings.HasSuffix(d.Name(), ".yaml") {
			dirs = append(dirs, filepath.Join(root, "controller/install/helm/agentgateway-crds/templates", d.Name()))
		}
	}
	v, err := crd.NewValidatorFromFiles(
		dirs...,
	)
	if err != nil {
		return nil, err
	}
	return v, nil
}
