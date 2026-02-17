package helmutils

// UninstallOpts is a set of typical options for a helm uninstall which can be passed in
// instead of requiring the caller to remember the helm cli flags.
type UninstallOpts struct {
	// KubeContext is the kubernetes context to use.
	KubeContext string

	// Namespace is the namespace to which the release was installed.
	Namespace string

	// ReleaseName is the name of the release to uninstall.
	ReleaseName string

	// ExtraArgs allows passing in arbitrary extra arguments to the uninstall.
	ExtraArgs []string
}

func (o UninstallOpts) all() []string {
	return append([]string{o.release()}, o.flags()...)
}

func (o UninstallOpts) flags() []string {
	args := []string{}
	appendIfNonEmpty := func(flagVal, flagName string) {
		if flagVal != "" {
			args = append(args, flagName, flagVal)
		}
	}

	appendIfNonEmpty(o.KubeContext, "--kube-context")
	appendIfNonEmpty(o.Namespace, "--namespace")

	for _, extraArg := range o.ExtraArgs {
		args = append(args, extraArg)
	}

	return args
}

func (o UninstallOpts) release() string {
	return o.ReleaseName
}
