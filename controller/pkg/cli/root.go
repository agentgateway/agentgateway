package cli

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/config"
	controllercmd "github.com/agentgateway/agentgateway/controller/pkg/cli/controller"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/costs"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	proxycmd "github.com/agentgateway/agentgateway/controller/pkg/cli/proxy"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/trace"
	cliversion "github.com/agentgateway/agentgateway/controller/pkg/cli/version"
)

func NewRootCmd() *cobra.Command {
	rootCmd := &cobra.Command{
		Use:   "agctl",
		Short: "agctl controls and inspects Agentgateway resources",
	}

	flag.AttachGlobalFlags(rootCmd)
	rootCmd.AddCommand(flag.BuildCobra(cliversion.Command))
	rootCmd.AddCommand(proxycmd.Command())
	rootCmd.AddCommand(controllercmd.Command())
	rootCmd.AddCommand(costs.Command())

	// Deprecated top-level aliases — delegate to the canonical subcommands.
	rootCmd.AddCommand(deprecatedAlias("config", "agctl proxy config", flag.BuildCobra(config.Command)))
	rootCmd.AddCommand(deprecatedAlias("trace", "agctl proxy trace", flag.BuildCobra(trace.Command)))

	return rootCmd
}

// deprecatedAlias wraps cmd so that running it prints a deprecation notice and
// then executes the same underlying logic.
func deprecatedAlias(use, canonical string, cmd *cobra.Command) *cobra.Command {
	cmd.Use = use
	cmd.Deprecated = fmt.Sprintf("use \"%s\" instead", canonical)
	cmd.Hidden = true
	return cmd
}

func Execute() {
	if err := NewRootCmd().Execute(); err != nil {
		os.Exit(1)
	}
}
