//go:build e2e

package install

import "fmt"

// Context contains the set of properties for a given installation of kgateway
type Context struct {
	InstallNamespace string

	// ChartType specifies which chart to install: "kgateway" or "agentgateway"
	// Defaults to "kgateway" for backwards compatibility
	// TODO: convert to enum
	ChartType string

	// ProfileValuesManifestFile points to the file that contains the set of Helm values for a given profile
	// This is intended to represent a set of "production recommendations" and is defined as a standalone
	// file, to guarantee that tests specify a file that contains these values
	// For a test to define Helm values that are unique to the test, use ValuesManifestFile
	ProfileValuesManifestFile string

	// ValuesManifestFile points to the file that contains the set of Helm values that are unique to this test
	ValuesManifestFile string

	// ExtraHelmArgs are additional Helm arguments
	ExtraHelmArgs []string
}

// ValidateInstallContext returns an error if the provided Context is invalid
func ValidateInstallContext(context *Context) error {
	return ValidateContext(context, validateValuesManifest)
}

func validateValuesManifest(name string, file string) error {
	if file == "" {
		return fmt.Errorf("%s must be provided in install.Context", name)
	}

	/*
		// TODO consider adding back helm value validation https://github.com/kgateway-dev/kgateway/issues/10483#issuecomment-2651621134
		values, err := testutils.BuildHelmValues(testutils.HelmValues{ValuesFile: file})
		if err != nil {
			return fmt.Errorf("failed to build helm values for %s: %w", name, err)
		}
		err = testutils.ValidateHelmValues(values)
		if err != nil {
			return fmt.Errorf("failed to validate helm values for %s: %w", name, err)
		}
	*/
	return nil
}

// ValidateContext returns an error if the provided Context is invalid
// This accepts a manifestValidator so that it can be overridden as needed.
func ValidateContext(context *Context, manifestValidator func(string, string) error) error {
	// We are intentionally restrictive, and expect a ProfileValuesManifestFile to be defined.
	// This is because we want all existing and future tests to rely on this concept
	if err := manifestValidator("ProfileValuesManifestFile", context.ProfileValuesManifestFile); err != nil {
		return err
	}

	if err := manifestValidator("ValuesManifestFile", context.ValuesManifestFile); err != nil {
		return err
	}

	return nil
}
