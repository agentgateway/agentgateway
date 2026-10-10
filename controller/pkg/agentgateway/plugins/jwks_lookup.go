package plugins

import (
	"fmt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
)

func resolveJWKSInlineForOwner(ctx PolicyCtx, owner jwks.RemoteJwksOwner) (string, error) {
	if ctx.JWKSLookup == nil {
		return `{"keys":[]}`, fmt.Errorf("jwks lookup is not configured")
	}
	inline, err := ctx.JWKSLookup.InlineForOwner(ctx.Krt, owner)
	if err != nil {
		// Keep authentication installed with no trusted keys while reporting the lookup failure.
		return `{"keys":[]}`, err
	}
	return inline, nil
}
