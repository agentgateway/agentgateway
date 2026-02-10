package plugins

import (
	"fmt"
	"net/url"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/jwks"
)

// resolveRemoteJWKSInline resolves a remote JWKS URI to an inline JWKS string by
// looking up the JWKS store ConfigMap and extracting the serialized JWKS JSON.
func resolveRemoteJWKSInline(ctx PolicyCtx, jwksURI string) (string, error) {
	if _, err := url.Parse(jwksURI); err != nil {
		return "", fmt.Errorf("invalid jwks url %w", err)
	}
	jwksStoreName := jwks.JwksConfigMapNamespacedName(jwksURI)
	if jwksStoreName == nil {
		return "", fmt.Errorf("jwks store hasn't been initialized")
	}
	jwksCM := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterObjectName(*jwksStoreName)))
	if jwksCM == nil {
		return "", fmt.Errorf("jwks ConfigMap %v isn't available", jwksStoreName)
	}
	jwksForURI, err := jwks.JwksFromConfigMap(jwksCM)
	if err != nil {
		return "", fmt.Errorf("error deserializing jwks ConfigMap %w", err)
	}
	inline, ok := jwksForURI[jwksURI]
	if !ok {
		return "", fmt.Errorf("jwks %s is not available in the jwks ConfigMap", jwksURI)
	}
	return inline, nil
}
