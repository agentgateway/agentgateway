//go:build e2e

package install

import "fmt"

// Context contains the set of properties for a given installation of agentgateway
type Context struct {
	InstallNamespace string

	ValuesManifestFile string

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

	return nil
}

// ValidateContext returns an error if the provided Context is invalid
// This accepts a manifestValidator so that it can be overridden as needed.
func ValidateContext(context *Context, manifestValidator func(string, string) error) error {
	if err := manifestValidator("ValuesManifestFile", context.ValuesManifestFile); err != nil {
		return err
	}

	return nil
}
