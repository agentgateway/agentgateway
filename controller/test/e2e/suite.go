//go:build e2e

package e2e

import (
	"context"
	"reflect"
	"sort"
	"strings"
	"testing"
)

type (
	TestingSuite any

	NewSuiteFunc func(ctx context.Context, testInstallation *TestInstallation) TestingSuite

	namedSuite struct {
		name     string
		newSuite NewSuiteFunc
	}

	orderedSuites struct {
		suites []namedSuite
	}

	suites struct {
		suites map[string]NewSuiteFunc
	}

	// A SuiteRunner is an interface that allows E2E tests to simply Register tests in one location and execute them
	// with Run.
	SuiteRunner interface {
		Run(ctx context.Context, t *testing.T, testInstallation *TestInstallation)
		Register(name string, newSuite NewSuiteFunc)
	}
)

var (
	_ SuiteRunner = new(orderedSuites)
	_ SuiteRunner = new(suites)
)

// NewSuiteRunner returns an implementation of TestRunner that will execute tests as specified
// in the ordered parameter.
//
// NOTE: it should be strongly preferred to use unordered tests. Only pass true to this function
// if there is a clear need for the tests to be ordered, and specify in a comment near the call
// to NewSuiteRunner why the tests need to be ordered.
func NewSuiteRunner(ordered bool) SuiteRunner {
	if ordered {
		return new(orderedSuites)
	}

	return new(suites)
}

func (o *orderedSuites) Run(ctx context.Context, t *testing.T, testInstallation *TestInstallation) {
	for _, namedTest := range o.suites {
		t.Run(namedTest.name, func(t *testing.T) {
			runTestingSuite(t, namedTest.name, namedTest.newSuite(ctx, testInstallation))
		})
	}
}

func (o *orderedSuites) Register(name string, newSuite NewSuiteFunc) {
	if o.suites == nil {
		o.suites = make([]namedSuite, 0)
	}
	o.suites = append(o.suites, namedSuite{
		name:     name,
		newSuite: newSuite,
	})
}

func (u *suites) Run(ctx context.Context, t *testing.T, testInstallation *TestInstallation) {
	// TODO(jbohanon) does some randomness need to be injected here to ensure they aren't run in the same order every time?
	// from https://goplay.tools/snippet/A-qqQCWkFaZ it looks like maps are not stable, but tend toward stability.
	for testName, newSuite := range u.suites {
		t.Run(testName, func(t *testing.T) {
			runTestingSuite(t, testName, newSuite(ctx, testInstallation))
		})
	}
}

func (u *suites) Register(name string, newSuite NewSuiteFunc) {
	if u.suites == nil {
		u.suites = make(map[string]NewSuiteFunc)
	}
	u.suites[name] = newSuite
}

type (
	suiteTSetter interface {
		SetT(*testing.T)
	}

	suiteSetup interface {
		SetupSuite()
	}

	suiteTearDown interface {
		TearDownSuite()
	}

	suiteBeforeTest interface {
		BeforeTest(suiteName, testName string)
	}

	suiteAfterTest interface {
		AfterTest(suiteName, testName string)
	}

	runnableSuite interface {
		RunSuite(t *testing.T, suiteName string)
	}
)

func runTestingSuite(t *testing.T, suiteName string, testSuite TestingSuite) {
	t.Helper()
	if runnable, ok := testSuite.(runnableSuite); ok {
		runnable.RunSuite(t, suiteName)
		return
	}

	setSuiteT(t, testSuite)

	if tearDown, ok := testSuite.(suiteTearDown); ok {
		defer func() {
			setSuiteT(t, testSuite)
			tearDown.TearDownSuite()
		}()
	}
	if setup, ok := testSuite.(suiteSetup); ok {
		setup.SetupSuite()
	}

	testNames := suiteTestNames(t, testSuite)
	if len(testNames) == 0 {
		t.Fatalf("suite %s has no Test* methods", suiteName)
	}

	for _, testName := range testNames {
		testName := testName
		t.Run(testName, func(t *testing.T) {
			setSuiteT(t, testSuite)
			defer setSuiteT(t, testSuite)

			if after, ok := testSuite.(suiteAfterTest); ok {
				defer after.AfterTest(suiteName, testName)
			}
			if before, ok := testSuite.(suiteBeforeTest); ok {
				before.BeforeTest(suiteName, testName)
			}

			reflect.ValueOf(testSuite).MethodByName(testName).Call(nil)
		})
	}
}

func setSuiteT(t *testing.T, testSuite TestingSuite) {
	if setter, ok := testSuite.(suiteTSetter); ok {
		setter.SetT(t)
	}
}

func suiteTestNames(t *testing.T, testSuite TestingSuite) []string {
	t.Helper()

	typ := reflect.TypeOf(testSuite)
	if typ == nil {
		t.Fatal("nil suite")
	}

	var names []string
	for i := 0; i < typ.NumMethod(); i++ {
		method := typ.Method(i)
		if !strings.HasPrefix(method.Name, "Test") {
			continue
		}
		if method.Type.NumIn() != 1 || method.Type.NumOut() != 0 {
			t.Fatalf("suite test method %s must have signature func()", method.Name)
		}
		names = append(names, method.Name)
	}
	sort.Strings(names)
	return names
}
