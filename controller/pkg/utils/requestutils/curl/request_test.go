package curl_test

import (
	"slices"
	"testing"

	"istio.io/istio/pkg/test/util/assert"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
)

func TestBuildArgs(t *testing.T) {
	tests := []struct {
		name     string
		option   curl.Option
		expected []string
	}{
		{
			name:     "VerboseOutput",
			option:   curl.VerboseOutput(),
			expected: []string{"-v"},
		},
		{
			name:     "Silent",
			option:   curl.Silent(),
			expected: []string{"-s"},
		},
		{
			name:     "WithBody",
			option:   curl.WithBody("body"),
			expected: []string{"--data-binary"},
		},
		{
			name:     "WithRetries",
			option:   curl.WithRetries(1, 1, 1),
			expected: []string{"--retry", "--retry-delay", "--retry-max-time"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			args := curl.BuildArgs(tt.option)
			for _, expected := range tt.expected {
				assert.Equal(t, true, slices.Contains(args, expected))
			}
		})
	}
}
