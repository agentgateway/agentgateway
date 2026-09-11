//go:build e2e

package e2e

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestConfiguredCoreChartExtraHelmArgs(t *testing.T) {
	t.Run("unset", func(t *testing.T) {
		t.Setenv(CoreChartExtraHelmArgsEnv, "")

		args, err := configuredCoreChartExtraHelmArgs()
		assert.NoError(t, err)
		assert.Nil(t, args)
	})

	t.Run("valid", func(t *testing.T) {
		t.Setenv(CoreChartExtraHelmArgsEnv, `["--set","feature.enabled=false"]`)

		args, err := configuredCoreChartExtraHelmArgs()
		assert.NoError(t, err)
		assert.Equal(t, []string{"--set", "feature.enabled=false"}, args)
	})

	t.Run("invalid", func(t *testing.T) {
		t.Setenv(CoreChartExtraHelmArgsEnv, "not-json")

		_, err := configuredCoreChartExtraHelmArgs()
		assert.Error(t, err)
	})
}
