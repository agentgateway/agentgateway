package config

import (
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	proxyconfig "github.com/agentgateway/agentgateway/controller/pkg/cli/proxy/config"
)

func Command() flag.Command {
	return proxyconfig.Command()
}
