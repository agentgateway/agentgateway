package profile

import (
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	proxyprofile "github.com/agentgateway/agentgateway/controller/pkg/cli/proxy/profile"
)

func Command() flag.Command {
	cmd := proxyprofile.Command()
	cmd.Deprecated = `use "agctl proxy profile" instead`
	cmd.Hidden = true
	return cmd
}
