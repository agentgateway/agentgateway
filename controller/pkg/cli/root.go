package cli

import (
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

	rootCmd.AddCommand(flag.BuildCobra(config.Command))
	rootCmd.AddCommand(flag.BuildCobra(trace.Command))

	return rootCmd
}

func Execute() {
	if err := NewRootCmd().Execute(); err != nil {
		os.Exit(1)
	}
}
