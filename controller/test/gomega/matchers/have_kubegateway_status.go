package matchers

import (
	"errors"
	"fmt"

	"github.com/onsi/gomega"
	"github.com/onsi/gomega/gstruct"
	"github.com/onsi/gomega/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// KubeGatewayRouteStatus defines the set of properties that we can validate from a k8s gateway RouteStatus
type KubeGatewayRouteStatus struct {

	// Custom is a generic matcher that can be applied to validate any other properties of a k8s gateway RouteStatus
	// Optional: If not provided, does not perform additional validation
	Custom types.GomegaMatcher
}

// HaveKubeGatewayRouteStatus produces a matcher that will match if the provided status matches the
// actual status
func HaveKubeGatewayRouteStatus(expected *KubeGatewayRouteStatus) types.GomegaMatcher {
	if expected == nil {
		// If no status is defined, we create a matcher that always succeeds
		return gstruct.Ignore()
	}
	expectedCustomMatcher := expected.Custom
	if expectedCustomMatcher == nil {
		// Default to an always accept matcher
		expectedCustomMatcher = gstruct.Ignore()
	}
	partialStatusMatchers := []types.GomegaMatcher{expectedCustomMatcher}

	// extend with other partial matchers here if needed...

	return &HaveKubeGatewayRouteStatusMatcher{
		Expected:      expected,
		statusMatcher: gomega.And(partialStatusMatchers...),
	}
}

type HaveKubeGatewayRouteStatusMatcher struct {
	Expected      *KubeGatewayRouteStatus
	statusMatcher types.GomegaMatcher
	// An internal utility for tracking whether we have evaluated this matcher
	// There is a comment within the Match method, outlining why we introduced this
	evaluated bool
}

func (m *HaveKubeGatewayRouteStatusMatcher) Match(actual any) (success bool, err error) {
	if m.evaluated {
		// Matchers are intended to be short-lived, and we have seen inconsistent behaviors
		// when evaluating the same matcher multiple times.
		// For example, the underlying http body matcher caches the response body, so if you are wrapping this
		// matcher in an Eventually, you need to create a new matcher each iteration.
		// This error is intended to help prevent developers hitting this edge case
		return false, errors.New("using the same matcher twice can lead to inconsistent behaviors")
	}
	m.evaluated = true

	val, ok := actual.(gwv1.RouteStatus)
	if !ok {
		return false, fmt.Errorf("matcher expected gwv1.RouteStatus, got %T", actual)
	}

	if ok, err := m.statusMatcher.Match(val); !ok {
		return false, err
	}

	return true, nil
}

func (m *HaveKubeGatewayRouteStatusMatcher) FailureMessage(actual any) (message string) {
	return fmt.Sprintf("%s \n%s",
		m.statusMatcher.FailureMessage(actual),
		informativeComparison(m.Expected, actual))
}

func (m *HaveKubeGatewayRouteStatusMatcher) NegatedFailureMessage(actual any) (message string) {
	return fmt.Sprintf("%s \n%s",
		m.statusMatcher.NegatedFailureMessage(actual),
		informativeComparison(m.Expected, actual))
}
