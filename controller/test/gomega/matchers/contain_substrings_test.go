package matchers_test

import (
	"testing"

	"istio.io/istio/pkg/test/util/assert"

	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestContainSubstrings(t *testing.T) {
	actualString := "this is the string"

	t.Run("contains substrings", func(t *testing.T) {
		tests := []struct {
			name               string
			expectedSubstrings []string
		}{
			{name: "empty list", expectedSubstrings: []string{}},
			{name: "empty string", expectedSubstrings: []string{""}},
			{name: "single substring", expectedSubstrings: []string{"this"}},
			{name: "multiple substrings", expectedSubstrings: []string{"the", "is", "this"}},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				matcher := matchers.ContainSubstrings(tt.expectedSubstrings)
				ok, err := matcher.Match(actualString)
				assert.NoError(t, err)
				assert.Equal(t, true, ok)
			})
		}
	})

	t.Run("does not contain substrings", func(t *testing.T) {
		tests := []struct {
			name               string
			expectedSubstrings []string
		}{
			{name: "missing substring", expectedSubstrings: []string{"missing"}},
			{name: "substring and missing substring", expectedSubstrings: []string{"this", "missing"}},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				matcher := matchers.ContainSubstrings(tt.expectedSubstrings)
				ok, err := matcher.Match(actualString)
				assert.NoError(t, err)
				assert.Equal(t, false, ok)
			})
		}
	})
}
