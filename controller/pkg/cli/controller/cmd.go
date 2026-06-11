package controller

import (
	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/controller/log"
)

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "controller",
		Short: "Inspect and manage the agentgateway controller",
		Long:  "Inspect and manage the agentgateway controller admin API.",
	}

	cmd.AddCommand(log.Command())

	return cmd
}
