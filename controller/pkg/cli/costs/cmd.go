package costs

import (
	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/catalog"
)

// Command is the deprecated "costs" alias of "catalog", kept for backward compatibility.
func Command() *cobra.Command {
	cmd := catalog.Command()
	cmd.Use = "costs"
	cmd.Deprecated = `use "agctl catalog" instead`
	return cmd
}
