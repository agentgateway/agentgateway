package config

import (
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	proxyconfig "github.com/agentgateway/agentgateway/controller/pkg/cli/proxy/config"
)

func Command() flag.Command {
	cmd := proxyconfig.Command()
	cmd.Deprecated = `use "agctl proxy config" instead`
	cmd.Hidden = true
	return cmd
}
