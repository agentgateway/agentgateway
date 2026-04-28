package config

import (
	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
)

func allCommand(common *commonFlags) flag.Command {
	return flag.Command{
		Use:   "all",
		Short: "Retrieve all Agentgateway configuration",
		Long:  "Retrieve all Agentgateway configuration.",
		Args: func(cmd *cobra.Command, args []string) error {
			return common.validateArgs(cmd, args)
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			source, err := loadConfigDumpSource(cmd.Context(), common, args)
			if err != nil {
				return err
			}
			printData(cmd.OutOrStdout(), common.outputFormat, source.ConfigDump)

			return err
		},
	}
}
