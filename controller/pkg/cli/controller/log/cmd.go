package log

import (
	"context"
	"fmt"
	"net/url"
	"strings"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/namespaces"
)

const controllerAdminPort = 9095

type flags struct {
	namespace           string
	controllerAdminPort int
	level               string
	set                 []string
}

func Command() *cobra.Command {
	f := &flags{controllerAdminPort: controllerAdminPort}

	cmd := &cobra.Command{
		Use:   "log",
		Short: "Get or set controller log levels",
		Long: `Get or set log levels on the agentgateway controller.

With no flags, prints the current log level for each component.

When multiple controller pods are running, all are targeted and output
is prefixed per pod. All pods are attempted even if one fails.`,
		Example: `agctl controller log                               # show current levels
agctl controller log --level debug                 # set all components to debug
agctl controller log --set reconciler=debug        # set a single component
agctl controller log --set reconciler=debug --set xds=info  # set multiple`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return run(cmd, f)
		},
	}

	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", namespaces.DefaultNamespace, "Namespace where the controller is running")
	cmd.Flags().IntVarP(&f.controllerAdminPort, "controller-admin-port", "p", f.controllerAdminPort, "Controller admin port")
	cmd.Flags().StringVar(&f.level, "level", "", "Set log level for all components (error|warn|info|debug|trace)")
	cmd.Flags().StringArrayVar(&f.set, "set", nil, "Set a component log level: component=level (may be repeated)")

	return cmd
}

func run(cmd *cobra.Command, f *flags) error {
	if f.level != "" && len(f.set) > 0 {
		return fmt.Errorf("--level and --set are mutually exclusive")
	}

	kubeClient, err := kubeutil.NewCLIClient()
	if err != nil {
		return err
	}

	pods, err := kubeutil.ResolveControllerPods(cmd.Context(), kubeClient, f.namespace)
	if err != nil {
		return err
	}

	path, err := buildPath(f)
	if err != nil {
		return err
	}

	// The controller /logging handler requires POST or PUT even for reads;
	// sending POST with no query params returns the current levels.
	return kubeutil.ForEachPod(cmd.Context(), pods, cmd.OutOrStdout(), func(ctx context.Context, pod kubeutil.Pod) (string, error) {
		out, err := kubeClient.AgentgatewayRequest(ctx, pod.Name, pod.Namespace, "POST", path, f.controllerAdminPort)
		if err != nil {
			return "", err
		}
		return string(out), nil
	})
}

// buildPath constructs the request path for the controller /logging endpoint.
// The controller accepts POST/PUT with:
//   - no params: returns current log levels
//   - ?level=<level>: sets all components to level
//   - ?<component>=<level>&...: sets specific components
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

	params := url.Values{}
	for _, s := range f.set {
		for part := range strings.SplitSeq(s, ",") {
			part = strings.TrimSpace(part)
			if part == "" {
				continue
			}
			kv := strings.SplitN(part, "=", 2)
			if len(kv) != 2 || kv[0] == "" {
				return "", fmt.Errorf("invalid --set value %q: expected component=level", part)
			}
			if err := validateLevel(kv[1]); err != nil {
				return "", fmt.Errorf("--set %s: %w", kv[0], err)
			}
			params.Set(kv[0], kv[1])
		}
	}

	return "logging?" + params.Encode(), nil
}

func validateLevel(level string) error {
	switch strings.ToLower(level) {
	case "error", "warn", "info", "debug", "trace":
		return nil
	default:
		return fmt.Errorf("unknown level %q; must be one of error|warn|info|debug|trace", level)
	}
}
