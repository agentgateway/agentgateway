package proxy

import (
	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/config"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/proxy/log"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/trace"
)

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "proxy",
		Short: "Inspect and manage the agentgateway proxy",
		Long:  "Inspect and manage the agentgateway proxy admin API.",
	}

	cmd.AddCommand(flag.BuildCobra(config.Command))
	cmd.AddCommand(flag.BuildCobra(trace.Command))
	cmd.AddCommand(log.Command())

	return cmd
}
