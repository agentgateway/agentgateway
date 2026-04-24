package cli

import (
	"os"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/config"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/trace"
)

func NewRootCmd() *cobra.Command {
	rootCmd := &cobra.Command{
		Use:   "agctl",
		Short: "agctl controls and inspects Agentgateway resources",
	}

	flag.AttachGlobalFlags(rootCmd)
	rootCmd.AddCommand(flag.BuildCobra(config.Command))
	rootCmd.AddCommand(flag.BuildCobra(trace.Command))

	return rootCmd
}

func Execute() {
	if err := NewRootCmd().Execute(); err != nil {
		os.Exit(1)
	}
}
