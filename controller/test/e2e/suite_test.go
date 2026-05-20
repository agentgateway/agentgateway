//go:build e2e

package e2e

import (
	"reflect"
	"testing"
)

func TestRunTestingSuiteUsesStandardSubtests(t *testing.T) {
	var events []string
	suite := &runnerTestSuite{events: &events}

	runTestingSuite(t, "Feature", suite)

	want := []string{
		"set TestRunTestingSuiteUsesStandardSubtests",
		"setup TestRunTestingSuiteUsesStandardSubtests",
		"set TestRunTestingSuiteUsesStandardSubtests/TestAlpha",
		"before Feature/TestAlpha",
		"test TestAlpha",
		"after Feature/TestAlpha",
		"set TestRunTestingSuiteUsesStandardSubtests/TestAlpha",
		"set TestRunTestingSuiteUsesStandardSubtests/TestBeta",
		"before Feature/TestBeta",
		"test TestBeta",
		"after Feature/TestBeta",
		"set TestRunTestingSuiteUsesStandardSubtests/TestBeta",
		"set TestRunTestingSuiteUsesStandardSubtests",
		"teardown TestRunTestingSuiteUsesStandardSubtests",
	}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events mismatch\nwant: %#v\n got: %#v", want, events)
	}
}

type runnerTestSuite struct {
	t      *testing.T
	events *[]string
}

func (s *runnerTestSuite) SetT(t *testing.T) {
	s.t = t
	*s.events = append(*s.events, "set "+t.Name())
}

func (s *runnerTestSuite) SetupSuite() {
	*s.events = append(*s.events, "setup "+s.t.Name())
}

func (s *runnerTestSuite) TearDownSuite() {
	*s.events = append(*s.events, "teardown "+s.t.Name())
}

func (s *runnerTestSuite) BeforeTest(suiteName, testName string) {
	*s.events = append(*s.events, "before "+suiteName+"/"+testName)
}

func (s *runnerTestSuite) AfterTest(suiteName, testName string) {
	*s.events = append(*s.events, "after "+suiteName+"/"+testName)
}

func (s *runnerTestSuite) TestBeta() {
	*s.events = append(*s.events, "test TestBeta")
}

func (s *runnerTestSuite) TestAlpha() {
	*s.events = append(*s.events, "test TestAlpha")
}
