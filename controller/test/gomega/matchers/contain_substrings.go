package matchers

import (
	"github.com/onsi/gomega"
	"github.com/onsi/gomega/gstruct"
	"github.com/onsi/gomega/types"
)

// ContainSubstrings produces a matcher that will match if all provided strings
// occur within the targeted string
func ContainSubstrings(substrings []string) types.GomegaMatcher {
	if len(substrings) == 0 {
		// If no substrings are defined, we create a matcher that always succeeds
		// If we do not this we will create an And matcher for 0 objects, which leads to a panic
		return gstruct.Ignore()
	}

	if len(substrings) == 1 {
		// If one substring is defined, we create a matcher for that substring
		// If we do not this we will create an And matcher for 1 object, which leads to a panic
		return gomega.ContainSubstring(substrings[0])
	}

	substringMatchers := make([]types.GomegaMatcher, 0, len(substrings))
	for i := range substrings {
		substringMatchers = append(substringMatchers, gomega.ContainSubstring(substrings[i]))
	}
	return gomega.And(substringMatchers...)
}
