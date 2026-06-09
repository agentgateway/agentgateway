package version

import (
	"fmt"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	pkgversion "github.com/agentgateway/agentgateway/controller/pkg/version"
)

func Command() flag.Command {
	return flag.Command{
		Use:   "version",
		Short: "Print agctl version information",
		Long:  "Print agctl version information.",
		RunE: func(cmd *cobra.Command, args []string) error {
			fmt.Fprintln(cmd.OutOrStdout(), pkgversion.String())
			return nil
		},
	}
}
