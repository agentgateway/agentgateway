package virtualkeys

import (
	"github.com/spf13/cobra"
)

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "virtualkeys",
		Short: "Migrate API key (virtual key) credentials",
	}
	cmd.AddCommand(toConfigMapCmd())
	return cmd
}
