package log

import (
	"context"
	"fmt"
	"net/url"
	"strings"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
)

const proxyAdminPort = 15000

type flags struct {
	namespace      string
	proxyAdminPort int
	level          string
	set            []string
}

func Command() *cobra.Command {
	f := &flags{proxyAdminPort: proxyAdminPort}

	cmd := &cobra.Command{
		Use:   "log [resource]",
		Short: "Get or set proxy log levels",
		Long: `Get or set log levels on the agentgateway proxy.

With no flags, prints the current active log filter directive.

The proxy uses Rust tracing-subscriber filter directives. Module names are
Rust crate paths, such as 'agentgateway::proxy'. See the agentgateway docs for
the full list of valid module paths. Level changes via '--set' are additive:
they append to the current directive rather than replacing it. Use '--level'
to reset to a clean global level.

When multiple pods back a resource, all are targeted and output is
prefixed per pod. All pods are attempted even if one fails.`,
		Example: `agctl proxy log                                      # show current directive
agctl proxy log --level debug                        # set global level
agctl proxy log --set agentgateway::proxy=debug      # set a single module
agctl proxy log --set agentgateway::proxy=debug,agentgateway::http=info`,
		Args:         cobra.MaximumNArgs(1),
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return run(cmd, f, args)
		},
	}

	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", "", "Namespace for proxy pod resolution")
	cmd.Flags().IntVarP(&f.proxyAdminPort, "proxy-admin-port", "p", f.proxyAdminPort, "Proxy admin port")
	cmd.Flags().StringVar(&f.level, "level", "", "Set global log level (error|warn|info|debug|trace|off)")
	cmd.Flags().StringArrayVar(&f.set, "set", nil, "Set module log level: module=level (may be repeated or comma-separated)")

	return cmd
}

func run(cmd *cobra.Command, f *flags, args []string) error {
	if f.level != "" && len(f.set) > 0 {
		return fmt.Errorf("--level and --set are mutually exclusive")
	}

	namespace, err := kubeutil.LoadNamespace(f.namespace)
	if err != nil {
		return err
	}

	kubeClient, err := kubeutil.NewCLIClient()
	if err != nil {
		return err
	}

	resourceName, err := kubeutil.ResolveResourceName(cmd.Context(), kubeClient, namespace, args)
	if err != nil {
		return err
	}

	pods, err := kubeutil.ResolvePodsForResource(cmd.Context(), kubeClient, resourceName, namespace)
	if err != nil {
		return err
	}

	path, err := buildPath(f)
	if err != nil {
		return err
	}

	return kubeutil.ForEachPod(cmd.Context(), pods, cmd.OutOrStdout(), func(ctx context.Context, pod kubeutil.Pod) (string, error) {
		out, err := kubeClient.AgentgatewayRequest(ctx, pod.Name, pod.Namespace, "POST", path, f.proxyAdminPort)
		if err != nil {
			return "", err
		}
		return string(out), nil
	})
}

// buildPath constructs the request path for the proxy /logging endpoint.
// The proxy accepts:
//   - no params: returns current level
//   - ?level=<level>: sets global level
//   - ?level=mod1:lvl1,mod2:lvl2: sets per-module levels (Rust tracing directive)
func buildPath(f *flags) (string, error) {
	if f.level != "" {
		if err := validateLevel(f.level); err != nil {
			return "", err
		}
		return "logging?level=" + url.QueryEscape(f.level), nil
	}

	if len(f.set) == 0 {
		return "logging", nil
	}

	var directives []string
	for _, s := range f.set {
		for part := range strings.SplitSeq(s, ",") {
			part = strings.TrimSpace(part)
			if part == "" {
				continue
			}
			if !strings.Contains(part, "=") {
				return "", fmt.Errorf("invalid --set value %q: expected module=level", part)
			}
			kv := strings.SplitN(part, "=", 2)
			if err := validateLevel(kv[1]); err != nil {
				return "", fmt.Errorf("--set %s: %w", kv[0], err)
			}
			// Proxy directive format uses colon separator: module:level
			directives = append(directives, kv[0]+":"+kv[1])
		}
	}

	if len(directives) == 0 {
		return "logging", nil
	}

	return "logging?level=" + url.QueryEscape(strings.Join(directives, ",")), nil
}

func validateLevel(level string) error {
	switch strings.ToLower(level) {
	case "error", "warn", "info", "debug", "trace", "off":
		return nil
	default:
		return fmt.Errorf("unknown level %q; must be one of error|warn|info|debug|trace|off", level)
	}
}
