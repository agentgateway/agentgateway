package helm

import (
	"embed"
)

var (
	//go:embed all:agentgateway
	AgentgatewayHelmChart embed.FS
)
