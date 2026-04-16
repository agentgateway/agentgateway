package plugins

import (
	"fmt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
)

func resolveJWKSInlineForOwner(ctx PolicyCtx, owner jwks.RemoteJwksOwner) (string, error) {
	if ctx.JWKSLookup == nil {
		return "", fmt.Errorf("jwks lookup is not configured")
	}
	return ctx.JWKSLookup.InlineForOwner(ctx.Krt, owner)
}
