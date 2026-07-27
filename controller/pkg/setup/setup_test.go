package setup

import (
	"testing"

	"github.com/stretchr/testify/require"

	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestResolveName(t *testing.T) {
	testCases := []struct {
		name          string
		optionValue   string
		settingsValue string
		want          string
	}{
		{
			name:          "option value overrides settings",
			optionValue:   "from-option",
			settingsValue: "from-settings",
			want:          "from-option",
		},
		{
			name:          "settings value used when option is empty",
			optionValue:   "",
			settingsValue: "from-settings",
			want:          "from-settings",
		},
		{
			name:          "settings default flows through when option is empty",
			optionValue:   "",
			settingsValue: wellknown.DefaultAgwClassName,
			want:          wellknown.DefaultAgwClassName,
		},
		{
			name:          "option wins over settings default",
			optionValue:   "custom-class",
			settingsValue: wellknown.DefaultAgwClassName,
			want:          "custom-class",
		},
	}
	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			require.Equal(t, tc.want, resolveName(tc.optionValue, tc.settingsValue))
		})
	}
}
