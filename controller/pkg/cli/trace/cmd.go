package trace

import (
	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	proxytrace "github.com/agentgateway/agentgateway/controller/pkg/cli/proxy/trace"
)

func Command() flag.Command {
	return proxytrace.Command()
}
