package helmutils

// InstallOpts is a set of typical options for a helm install which can be passed in
// instead of requiring the caller to remember the helm cli flags.
type InstallOpts struct {
	// KubeContext is the kubernetes context to use.
	KubeContext string

	// Namespace is the namespace to which the release will be installed.
	Namespace string

	// CreateNamespace controls whether to create the namespace or error if it doesn't exist.
	CreateNamespace bool

	// ValuesFiles is a list of absolute paths to YAML values for the installation. They will be
	// applied in the order they are specified.
	ValuesFiles []string

	// ExtraArgs allows passing in arbitrary extra arguments to the install.
	ExtraArgs []string

	// ReleaseName is the name of the release to install.
	ReleaseName string

	// Chart is the name of the chart to use. Ignored if ChartUri is set.
	Chart string
}

func (o InstallOpts) all() []string {
	return append([]string{o.release(), o.chart()}, o.flags()...)
}

func (o InstallOpts) flags() []string {
	args := []string{}
	appendIfNonEmpty := func(flagVal, flagName string) {
		if flagVal != "" {
			args = append(args, flagName, flagVal)
		}
	}

	appendIfNonEmpty(o.KubeContext, "--kube-context")
	appendIfNonEmpty(o.Namespace, "--namespace")
	if o.CreateNamespace {
		args = append(args, "--create-namespace")
	}
	for _, valsFile := range o.ValuesFiles {
		appendIfNonEmpty(valsFile, "--values")
	}
	for _, extraArg := range o.ExtraArgs {
		args = append(args, extraArg)
	}

	return args
}

func (o InstallOpts) chart() string {
	return o.Chart
}

func (o InstallOpts) release() string {
	return o.ReleaseName
}
