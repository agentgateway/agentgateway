//go:build e2e

package runtime

// RunSource identifies who/what triggered the test
type RunSource int

const (
	// LocalDevelopment signifies that the test is invoked locally
	LocalDevelopment RunSource = iota

	// PullRequest means that the test was invoked while running CI against a Pull Request
	PullRequest

	// NightlyTest means that the test was invoked while running CI as part of a Nightly operation
	NightlyTest
)
