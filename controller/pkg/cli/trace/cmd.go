package trace

import (
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	proxytrace "github.com/agentgateway/agentgateway/controller/pkg/cli/proxy/trace"
)

func Command() flag.Command {
	cmd := proxytrace.Command()
	cmd.Deprecated = `use "agctl proxy trace" instead`
	cmd.Hidden = true
	return cmd
}
