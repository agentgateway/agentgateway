package matchers

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"

	"github.com/onsi/gomega"
	"github.com/onsi/gomega/gstruct"
	"github.com/onsi/gomega/matchers"
	"github.com/onsi/gomega/types"
)

var _ types.GomegaMatcher = new(HaveHttpResponseMatcher)

// HttpResponse defines the set of properties that we can validate from an http.Response
type HttpResponse struct {
	// StatusCode is the expected status code for an http.Response
	// Required
	StatusCode int
	// Body is the expected response body for an http.Response
	// Body can be of type: {string, bytes, GomegaMatcher}
	// Optional: If not provided, defaults to an empty string
	Body any
	// Headers is the set of expected header values for an http.Response
	// Each header can be of type: {string, GomegaMatcher}
	// Optional: If not provided, does not perform header validation
	Headers map[string]any
	// NotHeaders is a list of headers that should not be present in the response
	// Optional: If not provided, does not perform header absence validation
	NotHeaders []string
	// Custom is a generic matcher that can be applied to validate any other properties of an http.Response
	// Optional: If not provided, does not perform additional validation
	Custom types.GomegaMatcher
	// IgnoreExitCode is the exit code that should be ignored when validating the response
	IgnoreExitCode int
}

func (r *HttpResponse) String() string {
	var bodyString string
	switch bodyMatcher := r.Body.(type) {
	case string:
		bodyString = bodyMatcher
	case []byte:
		bodyString = string(bodyMatcher)
	case types.GomegaMatcher:
		bodyString = fmt.Sprintf("%#v", bodyMatcher)
	}

	return fmt.Sprintf("HttpResponse{StatusCode: %d, Body: %s, Headers: %v, NotHeaders: %v, Custom: %v}",
		r.StatusCode, bodyString, r.Headers, r.NotHeaders, r.Custom)
}

// HaveHttpResponse returns a GomegaMatcher which validates that an http.Response contains
// particular expected properties (status, body..etc)
// If an expected body isn't specified, the body is not matched
func HaveHttpResponse(expected *HttpResponse) types.GomegaMatcher {
	expectedCustomMatcher := expected.Custom
	if expected.Custom == nil {
		// Default to an always accept matcher
		expectedCustomMatcher = gstruct.Ignore()
	}

	var partialResponseMatchers []types.GomegaMatcher
	partialResponseMatchers = append(partialResponseMatchers, &matchers.HaveHTTPStatusMatcher{
		Expected: []any{
			expected.StatusCode,
		},
	})
	if expected.Body != nil {
		partialResponseMatchers = append(partialResponseMatchers, &matchers.HaveHTTPBodyMatcher{
			Expected: expected.Body,
		})
	}
	for headerName, headerMatch := range expected.Headers {
		partialResponseMatchers = append(partialResponseMatchers, &matchers.HaveHTTPHeaderWithValueMatcher{
			Header: headerName,
			Value:  headerMatch,
		})
	}
	for _, headerName := range expected.NotHeaders {
		partialResponseMatchers = append(partialResponseMatchers, &NotHaveHTTPHeaderMatcher{
			Header: headerName,
		})
	}
	partialResponseMatchers = append(partialResponseMatchers, expectedCustomMatcher)

	return &HaveHttpResponseMatcher{
		Expected:        expected,
		responseMatcher: gomega.And(partialResponseMatchers...),
	}
}

type HaveHttpResponseMatcher struct {
	Expected *HttpResponse

	responseMatcher types.GomegaMatcher

	// An internal utility for tracking whether we have evaluated this matcher
	// There is a comment within the Match method, outlining why we introduced this
	evaluated bool
}

func (m *HaveHttpResponseMatcher) Match(actual any) (success bool, err error) {
	if m.evaluated {
		// Matchers are intended to be short-lived, and we have seen inconsistent behaviors
		// when evaluating the same matcher multiple times.
		// For example, the underlying http body matcher caches the response body, so if you are wrapping this
		// matcher in an Eventually, you need to create a new matcher each iteration.
		// This error is intended to help prevent developers hitting this edge case
		return false, errors.New("using the same matcher twice can lead to inconsistent behaviors")
	}
	m.evaluated = true

	if ok, matchErr := m.responseMatcher.Match(actual); !ok {
		return false, matchErr
	}

	return true, nil
}

func (m *HaveHttpResponseMatcher) FailureMessage(actual any) (message string) {
	return fmt.Sprintf("%s \n%s",
		m.responseMatcher.FailureMessage(actual),
		informativeComparison(m.Expected, actual))
}

func (m *HaveHttpResponseMatcher) NegatedFailureMessage(actual any) (message string) {
	return fmt.Sprintf("%s \n%s",
		m.responseMatcher.NegatedFailureMessage(actual),
		informativeComparison(m.Expected, actual))
}

// NotHaveHTTPHeaderMatcher is a matcher that checks if a header is not present in the HTTP response
type NotHaveHTTPHeaderMatcher struct {
	Header string
}

func (m *NotHaveHTTPHeaderMatcher) Match(actual any) (success bool, err error) {
	response, ok := actual.(*http.Response)
	if !ok {
		return false, fmt.Errorf("NotHaveHTTPHeaderMatcher expects an *http.Response, got %T", actual)
	}

	if response == nil {
		return false, errors.New("NotHaveHTTPHeaderMatcher matcher requires a non-nil *http.Response")
	}

	_, headerExists := response.Header[http.CanonicalHeaderKey(m.Header)]
	return !headerExists, nil
}

func (m *NotHaveHTTPHeaderMatcher) FailureMessage(actual any) string {
	response, ok := actual.(*http.Response)
	if !ok || response == nil {
		return fmt.Sprintf("Expected a valid *http.Response, got %T", actual)
	}

	return fmt.Sprintf("Expected HTTP response not to have header '%s', but it was present", m.Header)
}

func (m *NotHaveHTTPHeaderMatcher) NegatedFailureMessage(actual any) string {
	response, ok := actual.(*http.Response)
	if !ok || response == nil {
		return fmt.Sprintf("Expected a valid *http.Response, got %T", actual)
	}

	return fmt.Sprintf("Expected HTTP response to have header '%s', but it was not present", m.Header)
}

// informativeComparison returns a string which presents data to the user to help them understand why a failure occurred.
// The HaveHttpResponseMatcher uses an And matcher, which intentionally short-circuits and only
// logs the first failure that occurred.
// To help developers, we print more details in this function.
// NOTE: Printing the actual http.Response is challenging (since the body has already been read), so for now
// we do not print it.
func informativeComparison(expected, _ any) string {
	expectedJson, _ := json.MarshalIndent(expected, "", "  ")

	return fmt.Sprintf("\nexpected: %s", expectedJson)
}
