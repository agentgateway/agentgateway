package imports

import (
	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/imports/bedrock"
)

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "import",
		Short: "Import configuration data from external sources",
	}
	cmd.AddCommand(bedrock.Command())
	return cmd
}
