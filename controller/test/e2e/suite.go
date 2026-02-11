//go:build e2e

package e2e

import (
	"context"
	"testing"

	"github.com/stretchr/testify/suite"
)

type (
	NewSuiteFunc func(ctx context.Context, testInstallation *TestInstallation) suite.TestingSuite

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
			suite.Run(t, namedTest.newSuite(ctx, testInstallation))
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
			suite.Run(t, newSuite(ctx, testInstallation))
		})
	}
}

func (u *suites) Register(name string, newSuite NewSuiteFunc) {
	if u.suites == nil {
		u.suites = make(map[string]NewSuiteFunc)
	}
	u.suites[name] = newSuite
}
