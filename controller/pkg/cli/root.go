package cli

import (
	"os"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/config"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	resourcecmd "github.com/agentgateway/agentgateway/controller/pkg/cli/resource/cmd"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/trace"
	cliversion "github.com/agentgateway/agentgateway/controller/pkg/cli/version"

	// Import handlers so their init() functions register resource handlers.
	_ "github.com/agentgateway/agentgateway/controller/pkg/cli/resource/handlers"
)

func NewRootCmd() *cobra.Command {
	rootCmd := &cobra.Command{
		Use:   "agctl",
		Short: "agctl controls and inspects Agentgateway resources",
	}

	flag.AttachGlobalFlags(rootCmd)
	rootCmd.AddCommand(flag.BuildCobra(cliversion.Command))
	rootCmd.AddCommand(flag.BuildCobra(config.Command))
	rootCmd.AddCommand(flag.BuildCobra(trace.Command))

	// Resource-oriented commands (gwctl-compatible UX).
	rootCmd.AddCommand(resourcecmd.BuildGetCmd())
	rootCmd.AddCommand(resourcecmd.BuildDescribeCmd())
	rootCmd.AddCommand(resourcecmd.BuildAnalyzeCmd())

	return rootCmd
}

func Execute() {
	if err := NewRootCmd().Execute(); err != nil {
		os.Exit(1)
	}
}
