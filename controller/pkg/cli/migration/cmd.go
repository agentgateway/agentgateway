package migration

import (
	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/migration/virtualkeys"
)

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "migration",
		Short: "Migrate agentgateway resources to newer configurations",
		Long: `Migrate agentgateway resources to newer configurations.

Use subcommands to migrate specific resource kinds.`,
	}
	cmd.AddCommand(virtualkeys.Command())
	return cmd
}
