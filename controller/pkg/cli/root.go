package cli

import (
	"os"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/config"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/prerun"
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
	rootCmd.AddCommand(withVersionCheck(flag.BuildCobra(config.Command)))
	rootCmd.AddCommand(withVersionCheck(flag.BuildCobra(trace.Command)))

	return rootCmd
}

// withVersionCheck adds a PersistentPreRunE that warns when the client and
// controller versions differ. Errors are non-fatal: the command continues.
func withVersionCheck(cmd *cobra.Command) *cobra.Command {
	cmd.PersistentPreRunE = func(c *cobra.Command, args []string) error {
		prerun.CheckVersionMismatch(c.Context(), "", c.ErrOrStderr())
		return nil
	}
	return cmd
}

func Execute() {
	if err := NewRootCmd().Execute(); err != nil {
		os.Exit(1)
	}
}
